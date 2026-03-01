#![forbid(unsafe_code)]
use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, LazyLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::{Router, response::Json, routing::get};
use axum_prometheus::{PrometheusMetricLayer, metrics::gauge, metrics::histogram};

use clap::Parser;

use futures::stream::TryStreamExt;

use rand::RngExt;

use rtnetlink::{
    RouteMessageBuilder, new_connection,
    packet_route::route::{RouteAddress, RouteAttribute, RouteProtocol, RouteScope, RouteType},
    sys::AsyncSocket,
};

use serde_json::{Value, json};

use tokio::sync::Mutex;
use tokio::time::Duration;

use tower_http::compression::CompressionLayer;

static VERSION: LazyLock<String> =
    LazyLock::new(|| format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("BUILD_GIT_HASH")));

#[derive(Parser, Debug)]
#[command(version = VERSION.as_str(), about, long_about = None)]
struct Args {
    /// Target poll interval in minutes
    #[arg(long, default_value_t = 1)]
    target_poll_interval: u64,

    /// Target purge interval in minutes
    #[arg(long, default_value_t = 60)]
    target_purge_interval: u64,

    /// Port to listen on
    #[arg(long, default_value_t = 9198)]
    port: u16,
}

#[derive(Default)]
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
        let purge_interval = Duration::from_secs(60 * 60);

        self.targets.retain(|_, timestamp| {
            // Delete entries that have a timestamp older than one hour
            match timestamp.elapsed() {
                Ok(elapsed) => elapsed < purge_interval,
                Err(_) => false,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn add_and_get_targets() {
        let mut probe = ProbeTargets::default();

        probe.add_target("192.168.1.1".to_string());
        probe.add_target("10.0.0.1".to_string());

        let mut targets = probe.get_targets();
        targets.sort();

        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets,
            vec!["10.0.0.1".to_string(), "192.168.1.1".to_string(),]
        );
    }

    #[test]
    fn purge_removes_old_targets() {
        let mut probe = ProbeTargets::default();

        // Insert a fresh target
        probe.add_target("192.168.1.1".to_string());

        // Insert an old target by manually setting timestamp
        let old_time = SystemTime::now() - Duration::from_secs(60 * 60 + 1);
        probe.targets.insert("10.0.0.1".to_string(), old_time);

        probe.purge_old_targets();

        let targets = probe.get_targets();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], "192.168.1.1".to_string());
    }

    #[test]
    fn purge_keeps_recent_targets() {
        let mut probe = ProbeTargets::default();

        let recent_time = SystemTime::now() - Duration::from_secs(30);
        probe.targets.insert("10.0.0.1".to_string(), recent_time);

        probe.purge_old_targets();

        let targets = probe.get_targets();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], "10.0.0.1".to_string());
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
        if route.header.destination_prefix_length != 0 {
            continue;
        }

        let mut gateway = None;
        let mut oif = None;

        for attr in &route.attributes {
            match attr {
                RouteAttribute::Gateway(gw) => gateway = Some(gw),
                RouteAttribute::Oif(id) => oif = Some(id),
                _ => {}
            }
        }

        let gateway = match gateway {
            Some(gw) => gw,
            None => continue,
        };

        let gateway_str = match gateway {
            RouteAddress::Inet(addr) => addr.to_string(),
            RouteAddress::Inet6(addr) => {
                if addr.is_unicast_link_local() {
                    if let Some(oif) = oif {
                        format!("{}%{}", addr, oif)
                    } else {
                        addr.to_string()
                    }
                } else {
                    addr.to_string()
                }
            }

            _ => return Ok(None),
        };

        return Ok(Some(gateway_str));
    }

    Ok(None)
}

async fn collect_targets(
    probe_targets: Arc<Mutex<ProbeTargets>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut connection, handle, _) = new_connection()?;

    let _ = connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true);

    tokio::spawn(connection);

    let get_gateways_duration_time = Instant::now();

    let ip4_gw = get_gateways(&handle, IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to get IPv4 gateway: {}", e);
            None
        });

    histogram!("prometheus_sd_nexthop_get_gateway_duration", "family" => "ipv4")
        .record(get_gateways_duration_time.elapsed());

    let get_gateways_duration_time = Instant::now();

    let ip6_gw = get_gateways(&handle, IpAddr::V6(Ipv6Addr::UNSPECIFIED))
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to get IPv6 gateway: {}", e);
            None
        });

    histogram!("prometheus_sd_nexthop_get_gateway_duration", "family" => "ipv6")
        .record(get_gateways_duration_time.elapsed());

    let mut probe_targets = probe_targets.lock().await;

    if let Some(ip4) = ip4_gw {
        probe_targets.add_target(ip4);
    }

    if let Some(ip6) = ip6_gw {
        probe_targets.add_target(ip6);
    }

    Ok(())
}

async fn serve_targets(State(probe_targets): State<Arc<Mutex<ProbeTargets>>>) -> Json<Value> {
    let probe_targets = probe_targets.lock().await;

    let target_ips: Vec<String> = probe_targets.get_targets();

    // Place targets in JSON array as expected by Prometheus
    if target_ips.is_empty() {
        Json(json!([]))
    } else {
        Json(json!([{
            "targets": target_ips
        }]))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let targets_state = Arc::new(Mutex::new(ProbeTargets::default()));
    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind TCP listener on {}", addr));

    println!(
        "Starting prometheus-sd-nexthop {} server at {}",
        VERSION.as_str(),
        addr
    );

    tokio::spawn({
        // Target collection thread
        let targets_state = targets_state.clone();

        async move {
            loop {
                if let Err(e) = collect_targets(targets_state.clone()).await {
                    eprintln!("Failed to collect targets: {e}");
                }

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs_f64();
                gauge!("prometheus_sd_nexthop_targets_collection_timestamp_seconds").set(timestamp);

                tokio::time::sleep(Duration::from_secs(60 * args.target_poll_interval)).await;
            }
        }
    });

    tokio::spawn({
        // Target cleanup thread
        let targets_state = targets_state.clone();

        async move {
            loop {
                // Add an up-to-thirty-minute random delay to cleanup thread loop
                let random_delay = rand::rng().random_range(1..=60u64 * 30);

                tokio::time::sleep(Duration::from_secs(
                    60 * args.target_purge_interval + random_delay,
                ))
                .await;

                {
                    let mut probe_targets = targets_state.lock().await;
                    probe_targets.purge_old_targets();
                }

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs_f64();
                gauge!("prometheus_sd_nexthop_targets_purge_timestamp_seconds").set(timestamp);
            }
        }
    });

    gauge!("prometheus_sd_nexthop_build_info", "version" => env!("CARGO_PKG_VERSION"), "rev" => env!("BUILD_GIT_HASH")).set(1);

    let metrics_router = Router::new()
        .route("/metrics", get(|| async move { metric_handle.render() }))
        .layer(CompressionLayer::new());

    let app = Router::new()
        .route("/", get(serve_targets))
        .with_state(targets_state)
        .merge(metrics_router)
        .layer(prometheus_layer);

    axum::serve(listener, app).await.unwrap();

    Ok(())
}
