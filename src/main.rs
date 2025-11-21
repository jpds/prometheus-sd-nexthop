use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::{
    response::Json,
    routing::get,
    Router,
};

use futures::stream::TryStreamExt;

use rtnetlink::{
    RouteMessageBuilder, new_connection,
    packet_route::route::{
        RouteAddress, RouteAttribute, RouteProtocol, RouteScope, RouteType,
    },
    sys::AsyncSocket,
};

use serde_json::{Value, json};

async fn get_gateways(
    handle: &rtnetlink::Handle,
    ip_family: IpAddr,
) -> Option<String> {
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

    while let Some(route) = routes.try_next().await.ok()? {
        if route.header.destination_prefix_length == 0 {
            let has_gateway = route
                .attributes
                .iter()
                .any(|attr| matches!(attr, RouteAttribute::Gateway(_)));

            if has_gateway {
                for attr in &route.attributes {
                    match attr {
                        RouteAttribute::Gateway(gateway) => {
                            return match gateway {
                                RouteAddress::Inet(addr) => Some(addr.to_string()),
                                RouteAddress::Inet6(addr) => Some(addr.to_string()),
                                _ => None,
                            }
                        }
                        _ => ()
                    }
                }
            }
        }
    }

    None
}

async fn get_targets() -> Json<Value> {
    let (mut connection, handle, _) = new_connection().unwrap();

    let _ = connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true);

    tokio::spawn(connection);

    let ip4_gw = get_gateways(&handle, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))).await;

    let ip6_gw = get_gateways(&handle, IpAddr::V6(Ipv6Addr::UNSPECIFIED)).await;

    Json(json!({"targets": [ip4_gw, ip6_gw]}))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("[::]:3000").await.unwrap();

    let app = Router::new().route("/", get(get_targets));

    axum::serve(listener, app).await.unwrap();

    Ok(())
}
