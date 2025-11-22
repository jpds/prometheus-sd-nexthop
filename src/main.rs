#[forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::{Router, response::Json, routing::get};

use futures::stream::TryStreamExt;

use rtnetlink::{
    RouteMessageBuilder, new_connection,
    packet_route::route::{RouteAddress, RouteAttribute, RouteProtocol, RouteScope, RouteType},
    sys::AsyncSocket,
};

use serde_json::{Value, json};

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

                // Check if there's an outgoing interface (Oif) attribute
                if let Some(RouteAttribute::Oif(oif)) = route
                    .attributes
                    .iter()
                    .find(|attr| matches!(attr, RouteAttribute::Oif(_)))
                {
                    // Append the Oif value to the IPv6 address if it's IPv6
                    if let IpAddr::V6(_) = ip_family {
                        gateway_str = format!("{}%{}", gateway_str, oif);
                    }
                }

                return Ok(Some(gateway_str));
            }
        }
    }

    Ok(None)
}

async fn get_targets() -> Json<Value> {
    let (mut connection, handle, _) = new_connection().unwrap();

    let _ = connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true);

    tokio::spawn(connection);

    let ip4_gw = get_gateways(&handle, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)))
        .await
        .unwrap_or(None);
    let ip6_gw = get_gateways(&handle, IpAddr::V6(Ipv6Addr::UNSPECIFIED))
        .await
        .unwrap_or(None);

    // Collect the non-None gateway values in a list (Vec<Value>)
    let mut targets = Vec::new();

    if let Some(ip4) = ip4_gw {
        targets.push(Value::String(ip4));
    }

    if let Some(ip6) = ip6_gw {
        targets.push(Value::String(ip6));
    }

    // Return the JSON with the 'targets' array
    Json(json!([{
        "targets": targets
    }]))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("[::]:9198")
        .await
        .expect("Failed to bind TCP listener on [::]:9198");

    let app = Router::new().route("/", get(get_targets));

    axum::serve(listener, app).await.unwrap();

    Ok(())
}
