#![forbid(unsafe_code)]
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::State;
use axum::{Router, response::Json, routing::get};
use axum_prometheus::PrometheusMetricLayer;

use clap::Parser;

use futures::stream::TryStreamExt;

use rtnetlink::{
    RouteMessageBuilder, new_connection,
    packet_route::route::{RouteAddress, RouteAttribute, RouteProtocol, RouteScope, RouteType},
    sys::AsyncSocket,
};

use serde_json::{Value, json};

use tokio::sync::Mutex;
use tokio::time::Duration;

use tower_http::compression::CompressionLayer;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Target poll interval in minutes
    #[arg(long, default_value_t = 1)]
    target_poll_interval: u64,

    /// Target purge interval in minutes
    #[arg(long, default_value_t = 240)]
    target_purge_interval: u64,

    /// Port to listen on
    #[arg(long, default_value_t = 9198)]
    port: u16,
}

#[derive(Clone, Default)]
struct ProbeTargets {
    targets: HashMap<String, SystemTime>,
}

impl ProbeTargets {
    fn add_target(&mut self, target_ip: String) {
        self.targets.insert(target_ip, SystemTime::now());
    }

    fn get_targets(&self) -> Vec<String> {
        self.targets.keys().cloned().collect()
    }

    fn purge_old_targets(&mut self) {
        let purge_interval = Duration::from_secs(4 * 60 * 60);

        self.targets.retain(|_, timestamp| {
            // Delete entries that have a timestamp older than 4 hours ago
            match timestamp.elapsed() {
                Ok(elapsed) => elapsed < purge_interval,
                Err(_) => false,
            }
        });
    }
}

async fn get_gateways(
    handle: &rtnetlink::Handle,
    ip_family: IpAddr,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let route = match ip_family {
        IpAddr::V4(_) => RouteMessageBuilder::<Ipv4Addr>::new()
            .table_id(254)
            .protocol(RouteProtocol::Unspec)
            .scope(RouteScope::Universe)
            .kind(RouteType::Unspec)
            .build(),

        IpAddr::V6(_) => RouteMessageBuilder::<Ipv6Addr>::new()
            .table_id(254)
            .protocol(RouteProtocol::Unspec)
            .scope(RouteScope::Universe)
            .kind(RouteType::Unspec)
            .build(),
    };

    let mut routes = handle.route().get(route).execute();

    while let Some(route) = routes.try_next().await? {
        if route.header.destination_prefix_length == 0 {
            if let Some(RouteAttribute::Gateway(gateway)) = route
                .attributes
                .iter()
                .find(|attr| matches!(attr, RouteAttribute::Gateway(_)))
            {
                let mut gateway_str = match gateway {
                    RouteAddress::Inet(addr) => addr.to_string(),
                    RouteAddress::Inet6(addr) => addr.to_string(),
                    _ => return Ok(None),
                };

                // Find the outgoing interface numeric ID
                if let Some(RouteAttribute::Oif(oif)) = route
                    .attributes
                    .iter()
                    .find(|attr| matches!(attr, RouteAttribute::Oif(_)))
                {
                    // Append the outgoing interface ID to IPv6 addresses
                    if ip_family.is_ipv6() {
                        gateway_str = format!("{}%{}", gateway_str, oif);
                    }
                }

                return Ok(Some(gateway_str));
            }
        }
    }

    Ok(None)
}

async fn collect_targets(State(probe_targets): State<Arc<Mutex<ProbeTargets>>>) {
    let (mut connection, handle, _) = new_connection().unwrap();

    let _ = connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true);

    tokio::spawn(connection);

    let ip4_gw = get_gateways(&handle, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to get IPv4 gateway: {}", e);
            None
        });
    let ip6_gw = get_gateways(&handle, IpAddr::V6(Ipv6Addr::UNSPECIFIED))
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to get IPv6 gateway: {}", e);
            None
        });

    let mut probe_targets = probe_targets.lock().await;

    if let Some(ip4) = ip4_gw {
        probe_targets.add_target(ip4);
    }

    if let Some(ip6) = ip6_gw {
        probe_targets.add_target(ip6);
    }
}

async fn serve_targets(State(probe_targets): State<Arc<Mutex<ProbeTargets>>>) -> Json<Value> {
    let probe_targets = probe_targets.lock().await;

    let target_ips: Vec<String> = probe_targets.get_targets();

    // Place targets in JSON array as expected by Prometheus
    Json(json!([{
        "targets": target_ips
    }]))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let shared_targets_state = Arc::new(Mutex::new(ProbeTargets::default()));
    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let listener = tokio::net::TcpListener::bind(format!("[::]:{}", args.port))
        .await
        .expect(&format!(
            "Failed to bind TCP listener on [::]:{}",
            args.port
        ));

    println!(
        "Starting prometheus-sd-nexthop server at [::]:{}",
        args.port
    );

    tokio::spawn({
        let shared_targets_state = shared_targets_state.clone();

        async move {
            loop {
                collect_targets(State(shared_targets_state.clone())).await;

                tokio::time::sleep(tokio::time::Duration::from_secs(
                    60 * args.target_poll_interval,
                ))
                .await;
            }
        }
    });

    tokio::spawn({
        // Target cleanup thread
        let shared_targets_state = shared_targets_state.clone();

        async move {
            loop {
                {
                    let mut probe_targets = shared_targets_state.lock().await;
                    probe_targets.purge_old_targets();
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(
                    60 * args.target_purge_interval,
                ))
                .await;
            }
        }
    });

    let metrics_router = Router::new()
        .route("/metrics", get(|| async move { metric_handle.render() }))
        .layer(CompressionLayer::new());

    let app = Router::new()
        .route("/", get(serve_targets))
        .with_state(shared_targets_state)
        .merge(metrics_router)
        .layer(prometheus_layer);

    axum::serve(listener, app).await.unwrap();

    Ok(())
}
