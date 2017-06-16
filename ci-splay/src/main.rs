extern crate base32;
extern crate ci_capnp;
extern crate lz4;
extern crate num_cpus;
extern crate sha2;
extern crate tempfile;
extern crate thread_pool;

use std::fs;
use std::io;

use sha2::Digest;

use std::ascii::AsciiExt;
use std::io::Read;
use std::io::Write;

fn tools() -> (
    sha2::Sha256,
    lz4::EncoderBuilder,
) {
    (
        sha2::Sha256::default(),
        lz4::EncoderBuilder::new(),
    )
}

fn hash_compress_write_from_slice<W>(buf: &[u8], to: W) -> [u8; 256 / 8]
    where W: Write {
    let (mut hasher, lz4) = tools();
    let mut lz4 = lz4.build(to).expect("lz4 writer");

    hasher.input(buf);
    lz4.write_all(buf).expect("lz4 writing");
    lz4.finish();

    to_bytes(&hasher.result()[..])
}

fn hash_compress_write_from_reader<R, W>(mut from: R, to: W) -> (u64, [u8; 256 / 8])
    where W: Write,
          R: Read
{
    let (mut hasher, lz4) = tools();
    let mut lz4 = lz4.build(to).expect("lz4 writer");

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

    (total_read, to_bytes(&hasher.result()[..]))
}

fn to_bytes(slice: &[u8]) -> [u8; 256 / 8] {
    let mut hash = [0u8; 256 / 8];
    hash.clone_from_slice(slice);
    hash
}


fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    assert_eq!(2, args.len());
    let out_dir = args[1].to_string();

    {
        let alphabet_chars = "abcdefghijklmnopqrstuvwxyz234567";
        for first in alphabet_chars.chars() {
            for second in alphabet_chars.chars() {
                fs::create_dir_all(format!("{}/{}{}", out_dir, first, second)).expect("intermediate dir");
            }
        }
    }

    let (sender, pool) = thread_pool::Builder::new()
        .work_queue_capacity(num_cpus::get() * 2)
        .build();

    let mut stdin = io::stdin();
    while let Some(en) = ci_capnp::read_entry(&mut stdin).expect("capnp") {
        if 0 == en.len {
            continue;
        }

        let mut temp = tempfile::NamedTempFileOptions::new().suffix(".tmp").
            create_in(&out_dir)
            .expect("temp file");

        if en.len < 16 * 1024 * 1024 * 1024 {
            let mut buf = vec![0u8; en.len as usize];
            stdin.read_exact(&mut buf).expect("read");

            let out_dir = out_dir.clone();
            sender.send(move || {

                let hash = hash_compress_write_from_slice(&buf, &mut temp);

                complete(temp, &hash, out_dir.as_str());

            }).expect("offloading");

        } else {

            let file_data = (&mut stdin).take(en.len);
            let (total_read, hash) = hash_compress_write_from_reader(file_data, &mut temp);
            assert_eq!(en.len, total_read);

            complete(temp, &hash, out_dir.as_str());
        }
    }

    pool.shutdown();
    pool.await_termination();
}

fn complete(temp: tempfile::NamedTempFile, hash: &[u8], out_dir: &str) {
    let mut encoded_hash = base32::encode(base32::Alphabet::RFC4648 { padding: false }, hash);
    encoded_hash.make_ascii_lowercase();
    let written_to = format!("{}/{}/1-{}.lz4", out_dir, &encoded_hash[0..2], &encoded_hash[2..]);
    temp.persist(written_to).expect("rename");
}
