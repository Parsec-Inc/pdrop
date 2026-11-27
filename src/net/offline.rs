use super::{Msg, Peer, Policy, Route, State, Transfer, Who, clean, next_id};
use crate::{contacts, relay, secure};
use std::{
    io,
    sync::{
        Arc,
        mpsc::{Sender, channel},
    },
    thread,
};

pub fn queue(to: &Peer, me: &Who, text: &[u8]) -> Option<io::Result<String>> {
    let cfg = match &to.route {
        Route::Relay(c) => c,
        Route::Lan(_) => to.fallback.as_ref()?,
    };
    let key = contacts::get(to.uid)?;
    Some((|| -> io::Result<String> {
        let mut payload = clean(&me.name).into_bytes();
        payload.push(0);
        payload.extend_from_slice(text);
        let blob = secure::seal(&me.key, &key, &payload)?;
        relay::drop_msg(cfg, to.uid, &blob)?;
        Ok("na fila do relay".to_string())
    })())
}

pub fn inbox(tx: Sender<Msg>, policy: Arc<Policy>) -> relay::Offline {
    Arc::new(move |blob: Vec<u8>| {
        let Ok((key, payload)) = secure::open(&policy.key, &blob) else {
            return;
        };
        let Some(at) = payload.iter().position(|&b| b == 0) else {
            return;
        };
        let name = String::from_utf8_lossy(&payload[..at]).into_owned();
        let text = String::from_utf8_lossy(&payload[at + 1..]).into_owned();
        let (tx, policy) = (tx.clone(), policy.clone());
        thread::spawn(move || deliver(&tx, &policy, key, &name, text));
    })
}

fn deliver(tx: &Sender<Msg>, policy: &Policy, key: [u8; 32], name: &str, text: String) {
    let (id, uid) = (next_id(), secure::id_of(&key));
    let mut t = Transfer {
        id,
        peer: clean(name),
        label: "mensagem".into(),
        size: text.len() as u64,
        done: 0,
        up: false,
        state: State::Waiting,
        kind: 't',
        note: String::new(),
        src: String::new(),
        uid,
        t0: None,
    };
    if policy.trust.lock().unwrap().contains_key(&uid) {
        tx.send(Msg::Auto(t.clone())).ok();
    } else {
        let (rtx, rrx) = channel();
        tx.send(Msg::Ask(t.clone(), rtx)).ok();
        if !rrx.recv().unwrap_or(false) {
            return;
        }
    }
    contacts::learn(key);
    t.state = State::Active;
    tx.send(Msg::Start(t)).ok();
    tx.send(Msg::End(id, Ok(text))).ok();
}
