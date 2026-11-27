use std::io;

pub const MAX_BLOB: usize = 17 * 1024;

#[cfg(feature = "store")]
mod real {
    use super::*;
    use rusqlite::{Connection, params};
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    const PER_UID: i64 = 200;
    const TOTAL: i64 = 20_000;
    const TTL: i64 = 7 * 24 * 3600;

    pub struct Store(Mutex<Connection>);

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }

    fn fail(e: rusqlite::Error) -> io::Error {
        io::Error::other(e.to_string())
    }

    impl Store {
        pub fn open(path: &str) -> io::Result<Self> {
            let c = Connection::open(path).map_err(fail)?;
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 CREATE TABLE IF NOT EXISTS msgs(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     room TEXT NOT NULL,
                     to_uid TEXT NOT NULL,
                     ts INTEGER NOT NULL,
                     blob BLOB NOT NULL);
                 CREATE INDEX IF NOT EXISTS msgs_to ON msgs(room, to_uid);",
            )
            .map_err(fail)?;
            Ok(Store(Mutex::new(c)))
        }

        pub fn put(&self, room: &str, to: u128, blob: &[u8]) -> Result<(), &'static str> {
            let c = self.0.lock().unwrap();
            let to = format!("{to:x}");
            let _ = c.execute("DELETE FROM msgs WHERE ts < ?1", params![now() - TTL]);
            let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<i64, &'static str> {
                c.query_row(sql, p, |r| r.get(0)).map_err(|_| "erro")
            };
            if count("SELECT COUNT(*) FROM msgs", &[])? >= TOTAL
                || count(
                    "SELECT COUNT(*) FROM msgs WHERE room = ?1 AND to_uid = ?2",
                    &[&room, &to],
                )? >= PER_UID
            {
                return Err("cheia");
            }
            c.execute(
                "INSERT INTO msgs(room, to_uid, ts, blob) VALUES (?1, ?2, ?3, ?4)",
                params![room, to, now(), blob],
            )
            .map(|_| ())
            .map_err(|_| "erro")
        }

        pub fn take(&self, room: &str, to: u128, limit: i64) -> io::Result<Vec<(i64, Vec<u8>)>> {
            let c = self.0.lock().unwrap();
            let mut q = c
                .prepare(
                    "SELECT id, blob FROM msgs WHERE room = ?1 AND to_uid = ?2
                     ORDER BY id LIMIT ?3",
                )
                .map_err(fail)?;
            q.query_map(params![room, format!("{to:x}"), limit], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .map_err(fail)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(fail)
        }

        pub fn ack(&self, room: &str, to: u128, id: i64) -> io::Result<()> {
            self.0
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM msgs WHERE id = ?1 AND room = ?2 AND to_uid = ?3",
                    params![id, room, format!("{to:x}")],
                )
                .map(|_| ())
                .map_err(fail)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn queued_until_acked_and_capped_per_recipient() {
            let s = Store::open(":memory:").unwrap();
            let room = "a".repeat(64);
            s.put(&room, 1, b"um").unwrap();
            s.put(&room, 1, b"dois").unwrap();
            s.put(&room, 2, b"outro").unwrap();
            let got = s.take(&room, 1, 10).unwrap();
            let bodies: Vec<Vec<u8>> = got.iter().map(|(_, b)| b.clone()).collect();
            assert_eq!(bodies, [b"um".to_vec(), b"dois".to_vec()]);
            assert_eq!(s.take(&room, 1, 10).unwrap().len(), 2, "sem ack volta");
            s.ack(&room, 2, got[0].0).unwrap();
            assert_eq!(s.take(&room, 1, 10).unwrap().len(), 2, "ack de outro uid não apaga");
            s.ack(&room, 1, got[0].0).unwrap();
            assert_eq!(s.take(&room, 1, 10).unwrap().len(), 1);
            for _ in 0..PER_UID {
                let _ = s.put(&room, 3, b"x");
            }
            assert_eq!(s.put(&room, 3, b"x"), Err("cheia"));
        }
    }
}

#[cfg(feature = "store")]
pub use real::Store;

#[cfg(not(feature = "store"))]
#[allow(dead_code)]
pub struct Store;

#[cfg(not(feature = "store"))]
impl Store {
    pub fn open(_: &str) -> io::Result<Self> {
        Err(io::Error::other(
            "este pdrop-relay foi compilado sem o recurso `store` (cargo build --features store)",
        ))
    }

    pub fn put(&self, _: &str, _: u128, _: &[u8]) -> Result<(), &'static str> {
        Err("sem-armazenamento")
    }

    pub fn take(&self, _: &str, _: u128, _: i64) -> io::Result<Vec<(i64, Vec<u8>)>> {
        Ok(Vec::new())
    }

    pub fn ack(&self, _: &str, _: u128, _: i64) -> io::Result<()> {
        Ok(())
    }
}
