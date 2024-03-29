use std::fs;
use std::process;

use std::io::Read;
use std::path::PathBuf;

const TEST_PATH: &str = "tests/real";

fn path_of(name: &str) -> PathBuf {
    let mut bin_folder = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    bin_folder.push(name);
    bin_folder
}

fn run(name: &str) -> Vec<u8> {
    let mut gen = process::Command::new(path_of("ci-gen"))
        .current_dir(TEST_PATH)
        .arg(name)
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::inherit())
        .spawn()
        .expect("gen started");

    let dump = process::Command::new(path_of("ci-dump"))
        .arg("--drop-local-fs-details")
        .stdin(gen.stdout.take().unwrap())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::inherit())
        .output()
        .expect("dump finished");

    assert!(gen.wait().expect("wait").success());
    assert!(dump.status.success());

    dump.stdout
}

#[test]
fn everything() {
    for f in fs::read_dir(TEST_PATH).unwrap() {
        let f = f.unwrap();
        let name = f.file_name();
        let name = name.to_str().unwrap();
        if name.starts_with(".") {
            continue;
        }

        let now = run(name);
        let mut old = Vec::with_capacity(now.len());
        fs::File::open(format!("{}/.{}.yml", TEST_PATH, name))
            .expect(format!("reference file for {}", name).as_str())
            .read_to_end(&mut old)
            .unwrap();

        if now == old {
            continue;
        }

        let now = String::from_utf8(now).unwrap();
        let old = String::from_utf8(old).unwrap();

        for diff in diff::lines(old.as_str(), now.as_str()) {
            match diff {
                diff::Result::Left(l) => println!("-{}", l),
                diff::Result::Both(l, _) => {
                    // TODO: gross hack
                    if l.starts_with("        ") || l.starts_with(" - paths:") {
                        println!(" {}", l)
                    }
                }
                diff::Result::Right(r) => println!("+{}", r),
            }
        }
        panic!("{} failed (see above diff)", name);
    }
}
