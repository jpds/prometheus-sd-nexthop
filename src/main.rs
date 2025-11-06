use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::stream::TryStreamExt;

use rtnetlink::{
    RouteMessageBuilder, new_connection,
    packet_route::route::{
        RouteAddress, RouteAttribute, RouteMessage, RouteProtocol, RouteScope, RouteType,
    },
    sys::AsyncSocket,
};
use tokio::time::{Duration, sleep};

async fn get_routes(
    handle: &rtnetlink::Handle,
    ip_family: IpAddr,
) -> Result<(), Box<dyn std::error::Error>> {
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
            let has_gateway = route
                .attributes
                .iter()
                .any(|attr| matches!(attr, RouteAttribute::Gateway(_)));

            if has_gateway {
                println!("{:?}", get_route_gateway(&route).unwrap());
            }
        }
    }

    Ok(())
}

fn get_route_gateway(route: &RouteMessage) -> Option<RouteAddress> {
    for attr in &route.attributes {
        match attr {
            RouteAttribute::Gateway(gateway) => {
                return Some(gateway.clone());
            }
            _ => (),
        }
    }

    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut connection, handle, _) = new_connection().unwrap();

    let _ = connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true);

    tokio::spawn(connection);

    loop {
        // Fetch and print IPv4 routes
        if let Err(err) = get_routes(&handle, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))).await {
            eprintln!("Error fetching IPv4 routes: {}", err);
        }

        // Fetch and print IPv6 routes
        if let Err(err) = get_routes(&handle, IpAddr::V6(Ipv6Addr::UNSPECIFIED)).await {
            eprintln!("Error fetching IPv6 routes: {}", err);
        }

        sleep(Duration::from_secs(10)).await;
    }
}
