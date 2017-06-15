extern crate base32;
extern crate ci_capnp;
extern crate lz4;
extern crate sha2;
extern crate tempfile;

use std::fs;
use std::io;

use sha2::Digest;

use std::ascii::AsciiExt;
use std::io::Read;
use std::io::Write;

fn hash_compress_write<R, W>(mut from: R, to: W) -> (u64, [u8; 256 / 8])
    where W: Write,
          R: Read
{
    let mut hasher = sha2::Sha256::default();

    let mut lz4 = lz4::EncoderBuilder::new()
        .build(to).expect("lz4 writer");


    let mut total_read = 0u64;
    loop {
        let mut buf = [0u8; 4096 * 16];

        let read = from.read(&mut buf).expect("reading");
        if 0 == read {
            break
        }

        total_read += read as u64;

        hasher.input(&buf[0..read]);
        lz4.write_all(&buf[0..read]).expect("lz4 written");
    }
    let (_, result) = lz4.finish();
    result.expect("lz4 finished");

    let mut hash = [0u8; 256 / 8];
    hash.clone_from_slice(&hasher.result()[..]);

    (total_read, hash)
}


fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    assert_eq!(2, args.len());
    let ref out_dir = args[1];

    // Not really, 'cos we lowercase it!
    let alphabet = base32::Alphabet::RFC4648 { padding: false };

    {
        let alphabet_chars = "abcdefghijklmnopqrstuvwxyz234567";
        for first in alphabet_chars.chars() {
            for second in alphabet_chars.chars() {
                fs::create_dir_all(format!("{}/{}{}", out_dir, first, second)).expect("intermediate dir");
            }
        }
    }

    let mut stdin = io::stdin();
    while let Some(en) = ci_capnp::read_entry(&mut stdin).expect("") {
        if 0 == en.len {
            continue;
        }

        let mut temp = tempfile::NamedTempFileOptions::new().suffix(".tmp").
            create_in(out_dir)
            .expect("temp file");

        let file_data = (&mut stdin).take(en.len);
        let (total_read, hash) = hash_compress_write(file_data, &mut temp);
        assert_eq!(en.len, total_read);
        let mut encoded_hash = base32::encode(alphabet, &hash);
        encoded_hash.make_ascii_lowercase();

        let written_to = format!("{}/{}/1-{}.lz4", out_dir, &encoded_hash[0..2], &encoded_hash[2..]);
        temp.persist(written_to).expect("rename");
    }
}
