use snow::{Builder, TransportState};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
};

const PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const SEALED: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";
pub const SEAL_MAX: usize = 16 * 1024;
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
    pub remote_key: [u8; 32],
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
    let remote_key: [u8; 32] = hs
        .get_remote_static()
        .ok_or_else(|| err("sem identidade"))?
        .try_into()
        .map_err(|_| err("chave inválida"))?;
    let remote = id_of(&remote_key);
    Ok(Secure {
        tcp,
        ts: hs.into_transport_mode().map_err(err)?,
        rbuf: vec![],
        rpos: 0,
        remote,
        remote_key,
    })
}

pub fn seal(my_key: &[u8; 32], their_public: &[u8; 32], msg: &[u8]) -> io::Result<Vec<u8>> {
    if msg.len() > SEAL_MAX {
        return Err(err("mensagem grande demais"));
    }
    let mut hs = Builder::new(SEALED.parse().unwrap())
        .local_private_key(my_key)
        .map_err(err)?
        .remote_public_key(their_public)
        .map_err(err)?
        .build_initiator()
        .map_err(err)?;
    let mut out = vec![0u8; msg.len() + 256];
    let n = hs.write_message(msg, &mut out).map_err(err)?;
    out.truncate(n);
    Ok(out)
}

pub fn open(my_key: &[u8; 32], blob: &[u8]) -> io::Result<([u8; 32], Vec<u8>)> {
    let mut hs = Builder::new(SEALED.parse().unwrap())
        .local_private_key(my_key)
        .map_err(err)?
        .build_responder()
        .map_err(err)?;
    let mut out = vec![0u8; blob.len()];
    let n = hs.read_message(blob, &mut out).map_err(err)?;
    out.truncate(n);
    let from = hs
        .get_remote_static()
        .ok_or_else(|| err("sem identidade"))?
        .try_into()
        .map_err(|_| err("chave inválida"))?;
    Ok((from, out))
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

    #[test]
    fn sealed_message_is_for_one_recipient_and_names_the_sender() {
        let ((ka, pa), (kb, pb), (kc, _)) = (generate(), generate(), generate());
        let blob = seal(&ka, &pb, "olá".as_bytes()).unwrap();
        assert_eq!(open(&kb, &blob).unwrap(), (pa, "olá".as_bytes().to_vec()));
        assert!(open(&kc, &blob).is_err());
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&kb, &tampered).is_err());
        assert!(seal(&ka, &pb, &[0u8; SEAL_MAX + 1]).is_err());
    }
}
