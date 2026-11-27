use crate::{
    config::{hex, load_setting, save_setting, unhex},
    secure::id_of,
};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const MAX: usize = 500;

type Book = HashMap<u128, [u8; 32]>;

fn book() -> &'static Mutex<Book> {
    static BOOK: OnceLock<Mutex<Book>> = OnceLock::new();
    BOOK.get_or_init(|| {
        let mut m = Book::new();
        if !cfg!(test) {
            for l in load_setting("chaves").unwrap_or_default().lines() {
                if let Some(k) = unhex(l.trim()) {
                    m.insert(id_of(&k), k);
                }
            }
        }
        Mutex::new(m)
    })
}

pub fn get(uid: u128) -> Option<[u8; 32]> {
    book().lock().unwrap().get(&uid).copied()
}

pub fn learn(key: [u8; 32]) {
    let mut b = book().lock().unwrap();
    let uid = id_of(&key);
    if b.get(&uid) == Some(&key) || (b.len() >= MAX && !b.contains_key(&uid)) {
        return;
    }
    b.insert(uid, key);
    if !cfg!(test) {
        let lines: Vec<String> = b.values().map(|k| hex(k)).collect();
        save_setting("chaves", &lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_keys_are_found_by_their_uid() {
        let (_, public) = crate::secure::generate();
        assert_eq!(get(id_of(&public)), None);
        learn(public);
        assert_eq!(get(id_of(&public)), Some(public));
    }
}
