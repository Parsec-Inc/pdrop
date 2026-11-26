use super::{DISCOVERY_PORT, Peer, Peers, Policy, Route, VIS_HIDDEN, VIS_TRUSTED, clean};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};

#[allow(clippy::too_many_arguments)]
pub fn discover(
    id: u64,
    uid: u128,
    name: Arc<Mutex<String>>,
    port: u16,
    color: Arc<AtomicU8>,
    policy: Arc<Policy>,
    peers: Peers,
    direct: Vec<IpAddr>,
) -> io::Result<()> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.set_broadcast(true)?;
    sock.bind(&SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT)).into())?;
    let rx: UdpSocket = sock.into();
    let tx = rx.try_clone()?;
    let reply = rx.try_clone()?;
    let start = Instant::now();
    let pings = peers.clone();
    let beacon = {
        let (color, name, policy) = (color.clone(), name.clone(), policy.clone());
        move || {
            format!(
                "PDROP3|{id:x}|{port}|{}|{}|{uid:x}|{}",
                color.load(Ordering::Relaxed),
                policy.vis.load(Ordering::Relaxed),
                name.lock().unwrap()
            )
        }
    };
    let b2 = beacon.clone();
    let sweep_beacon = beacon.clone();
    let tx_policy = policy.clone();
    let sweep_policy = policy.clone();
    let sweep = UdpSocket::bind(("0.0.0.0", 0))?;
    thread::spawn(move || {
        loop {
            if sweep_policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN {
                sleep(Duration::from_millis(500));
                continue;
            }
            let beacon = sweep_beacon();
            for ip in subnet_hosts() {
                let _ = sweep.send_to(beacon.as_bytes(), (ip, DISCOVERY_PORT));
                sleep(Duration::from_millis(2));
            }
            sleep(Duration::from_millis(1500));
        }
    });
    thread::spawn(move || {
        loop {
            if tx_policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN {
                sleep(Duration::from_millis(500));
                continue;
            }
            let send = |ip: IpAddr, msg: &str| {
                let _ = tx.send_to(msg.as_bytes(), (ip, DISCOVERY_PORT));
            };
            let beacon = beacon();
            send(Ipv4Addr::BROADCAST.into(), &beacon);
            direct.iter().for_each(|&ip| send(ip, &beacon));
            let ping = format!("PING|{id:x}|{}", start.elapsed().as_micros());
            let known: Vec<IpAddr> = pings
                .lock()
                .unwrap()
                .values()
                .filter_map(|p| p.route.lan_ip())
                .collect();
            for ip in known {
                send(ip, &beacon);
                send(ip, &ping);
            }
            sleep(Duration::from_millis(1500));
        }
    });
    thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = rx.recv_from(&mut buf) {
            let Ok(s) = std::str::from_utf8(&buf[..n]) else {
                continue;
            };
            let hidden = policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN;
            let f: Vec<&str> = s.splitn(7, '|').collect();
            match f.as_slice() {
                ["PING", _, us, ..] if !hidden => {
                    let _ = reply.send_to(
                        format!("PONG|{id:x}|{us}").as_bytes(),
                        (from.ip(), DISCOVERY_PORT),
                    );
                }
                ["PONG", rid, us, ..] => {
                    let (Ok(rid), Ok(us)) = (u64::from_str_radix(rid, 16), us.parse::<u128>())
                    else {
                        continue;
                    };
                    let ms = start.elapsed().as_micros().saturating_sub(us) as f64 / 1000.0;
                    if let Some(p) = peers.lock().unwrap().get_mut(&rid) {
                        p.rtt = Some(p.rtt.map_or(ms, |old| old * 0.7 + ms * 0.3));
                    }
                }
                ["PDROP3", pid, port, col, vis, uid, name] => {
                    let (Ok(pid), Ok(port), Ok(col), Ok(vis), Ok(puid)) = (
                        u64::from_str_radix(pid, 16),
                        port.parse::<u16>(),
                        col.parse::<u8>(),
                        vis.parse::<u8>(),
                        u128::from_str_radix(uid, 16),
                    ) else {
                        continue;
                    };
                    if pid == id {
                        continue;
                    }
                    if !shown(
                        vis,
                        policy.vis.load(Ordering::Relaxed),
                        policy.trust.lock().unwrap().contains_key(&puid),
                    ) {
                        continue;
                    }
                    let (now, route) =
                        (Instant::now(), Route::Lan(SocketAddr::new(from.ip(), port)));
                    let new_on_lan = upsert(&peers, pid, clean(name), route, col, puid, now);
                    if new_on_lan && !hidden {
                        let _ = reply.send_to(b2().as_bytes(), (from.ip(), DISCOVERY_PORT));
                    }
                }
                _ => {}
            }
        }
    });
    Ok(())
}

const LAN_FRESH: Duration = Duration::from_secs(4);

pub fn upsert(
    peers: &Peers,
    id: u64,
    name: String,
    route: Route,
    color: u8,
    uid: u128,
    now: Instant,
) -> bool {
    let lan = matches!(route, Route::Lan(_));
    let mut g = peers.lock().unwrap();
    let Some(p) = g.get_mut(&id) else {
        g.insert(
            id,
            Peer {
                id,
                name,
                route,
                fallback: None,
                seen: now,
                born: now,
                rtt: None,
                color,
                uid,
            },
        );
        return lan;
    };
    let was_lan = matches!(p.route, Route::Lan(_));
    if !lan && p.uid != uid {
        return false;
    }
    match &route {
        Route::Relay(cfg) if was_lan && now.duration_since(p.seen) < LAN_FRESH => {
            p.fallback = Some(cfg.clone());
            return false;
        }
        Route::Relay(_) => (p.rtt, p.fallback) = (None, None),
        Route::Lan(_) if p.uid != uid => p.fallback = None,
        Route::Lan(_) => {
            if let Route::Relay(cfg) = &p.route {
                p.fallback = Some(cfg.clone());
            }
        }
    }
    (p.name, p.route, p.seen, p.color, p.uid) = (name, route, now, color, uid);
    lan && !was_lan
}

pub fn shown(theirs: u8, mine: u8, trusted: bool) -> bool {
    trusted || (theirs != VIS_TRUSTED && mine != VIS_TRUSTED)
}

fn subnet_hosts() -> Vec<Ipv4Addr> {
    let mut v = vec![];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        let loopback = i.is_loopback();
        let if_addrs::IfAddr::V4(a) = i.addr else {
            continue;
        };
        let (ip, mask) = (u32::from(a.ip), u32::from(a.netmask));
        if loopback || mask.leading_zeros() > 0 || mask.count_zeros() > 10 || mask == 0 {
            continue;
        }
        let net = ip & mask;
        v.extend(
            (1..!mask)
                .map(|h| Ipv4Addr::from(net | h))
                .filter(|&h| h != a.ip),
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::VIS_ALL, relay};

    #[test]
    fn lan_has_priority_over_relay() {
        let peers = Peers::default();
        let lan = Route::Lan(([127, 0, 0, 1], 1).into());
        let t0 = Instant::now();
        let put = |route, uid, at| upsert(&peers, 1, "a".into(), route, 0, uid, at);
        let relay = || {
            Route::Relay(Arc::new(relay::Config {
                addr: String::new(),
                room: String::new(),
            }))
        };
        let is_lan = || matches!(peers.lock().unwrap()[&1].route, Route::Lan(_));

        assert!(!put(relay(), 7, t0));
        assert!(put(lan.clone(), 7, t0), "veio do relay e apareceu na LAN");
        assert!(!put(lan, 7, t0));
        let soon = t0 + Duration::from_secs(1);
        put(relay(), 7, soon);
        assert!(is_lan());
        assert!(peers.lock().unwrap()[&1].fallback.is_some());
        assert_eq!(peers.lock().unwrap()[&1].seen, t0);
        put(relay(), 8, t0 + LAN_FRESH);
        assert!(is_lan(), "outra identidade não assume");
        put(relay(), 7, t0 + LAN_FRESH);
        assert!(!is_lan());
    }

    #[test]
    fn visibility_rule() {
        assert!(shown(VIS_ALL, VIS_ALL, false));
        assert!(!shown(VIS_TRUSTED, VIS_ALL, false));
        assert!(shown(VIS_TRUSTED, VIS_ALL, true));
        assert!(!shown(VIS_ALL, VIS_TRUSTED, false));
        assert!(shown(VIS_ALL, VIS_TRUSTED, true));
    }
}
