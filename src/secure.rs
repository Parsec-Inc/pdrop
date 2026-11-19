use snow::{Builder, TransportState};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
};

const PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const CHUNK: usize = 65000;

fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

pub fn generate() -> ([u8; 32], [u8; 32]) {
    let k = Builder::new(PARAMS.parse().unwrap())
        .generate_keypair()
        .expect("sem fonte de aleatoriedade");
    (k.private.try_into().unwrap(), k.public.try_into().unwrap())
}

pub fn id_of(public: &[u8]) -> u128 {
    u128::from_be_bytes(blake3::hash(public).as_bytes()[..16].try_into().unwrap())
}

pub fn code(a: u128, b: u128) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    let h = blake3::hash(&[lo.to_be_bytes(), hi.to_be_bytes()].concat());
    format!(
        "{:04}",
        u32::from_be_bytes(h.as_bytes()[..4].try_into().unwrap()) % 10_000
    )
}

fn write_frame(t: &mut TcpStream, d: &[u8]) -> io::Result<()> {
    t.write_all(&(d.len() as u16).to_be_bytes())?;
    t.write_all(d)
}

fn read_frame(t: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 2];
    match t.read(&mut len[..1])? {
        0 => return Ok(None),
        _ => t.read_exact(&mut len[1..])?,
    }
    let mut f = vec![0u8; u16::from_be_bytes(len) as usize];
    t.read_exact(&mut f)?;
    Ok(Some(f))
}

pub struct Secure {
    tcp: TcpStream,
    ts: TransportState,
    rbuf: Vec<u8>,
    rpos: usize,
    pub remote: u128,
}

impl Secure {
    pub fn tcp(&self) -> &TcpStream {
        &self.tcp
    }
}

pub fn handshake(mut tcp: TcpStream, key: &[u8; 32], initiator: bool) -> io::Result<Secure> {
    let b = Builder::new(PARAMS.parse().unwrap())
        .local_private_key(key)
        .map_err(err)?;
    let mut hs = if initiator {
        b.build_initiator()
    } else {
        b.build_responder()
    }
    .map_err(err)?;
    let mut buf = vec![0u8; 65535];
    while !hs.is_handshake_finished() {
        if hs.is_my_turn() {
            let n = hs.write_message(&[], &mut buf).map_err(err)?;
            write_frame(&mut tcp, &buf[..n])?;
        } else {
            let f = read_frame(&mut tcp)?.ok_or_else(|| err("conexão fechada no handshake"))?;
            hs.read_message(&f, &mut buf).map_err(err)?;
        }
    }
    let remote = id_of(
        hs.get_remote_static()
            .ok_or_else(|| err("sem identidade"))?,
    );
    Ok(Secure {
        tcp,
        ts: hs.into_transport_mode().map_err(err)?,
        rbuf: vec![],
        rpos: 0,
        remote,
    })
}

impl Read for Secure {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.rpos >= self.rbuf.len() {
            let Some(f) = read_frame(&mut self.tcp)? else {
                return Ok(0);
            };
            self.rbuf = vec![0u8; f.len()];
            let n = self.ts.read_message(&f, &mut self.rbuf).map_err(err)?;
            self.rbuf.truncate(n);
            self.rpos = 0;
        }
        let n = out.len().min(self.rbuf.len() - self.rpos);
        out[..n].copy_from_slice(&self.rbuf[self.rpos..self.rpos + n]);
        self.rpos += n;
        Ok(n)
    }
}

impl Write for Secure {
    fn write(&mut self, d: &[u8]) -> io::Result<usize> {
        let d = &d[..d.len().min(CHUNK)];
        let mut buf = vec![0u8; d.len() + 16];
        let n = self.ts.write_message(d, &mut buf).map_err(err)?;
        write_frame(&mut self.tcp, &buf[..n])?;
        Ok(d.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn roundtrip_and_identity() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let ((ka, pa), (kb, pb)) = (generate(), generate());
        let srv = std::thread::spawn(move || {
            let mut s = handshake(l.accept().unwrap().0, &kb, false).unwrap();
            let mut got = vec![];
            s.read_to_end(&mut got).unwrap();
            (s.remote, got)
        });
        let mut c = handshake(TcpStream::connect(addr).unwrap(), &ka, true).unwrap();
        assert_eq!(c.remote, id_of(&pb));
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        c.write_all(&data).unwrap();
        drop(c);
        let (remote, got) = srv.join().unwrap();
        assert_eq!((remote, got), (id_of(&pa), data));
        assert_eq!(code(1, 2), code(2, 1));
    }
}
