extern crate capnp;
extern crate ci_capnp;
extern crate clap;
extern crate regex;

use std::io;
use std::process;

use clap::{Arg, App, SubCommand};

use std::io::BufRead;
use std::io::Read;
use std::io::Write;

fn with_entries<
    R: io::Read,
    F: FnMut(&mut R, &ci_capnp::FileEntry) -> io::Result<()>
>(
    mut from: &mut R,
    mut work: F
) -> bool {

    loop {
        match ci_capnp::read_entry(&mut from) {
            Ok(None) => return true,
            Ok(Some(entry)) => if let Err(e) = work(&mut from, &entry) {
                let _ = write!(io::stderr(), "fatal: command error while processing '{}': {}\n",
                        join_backwards(&entry.paths, "/ /"),
                        e);
                return false;
            },
            Err(e) => match e.kind {
                capnp::ErrorKind::Failed => {
                    let _ = write!(io::stderr(), "fatal: capnp failure parsing stream: {}\n", e);
                    return false;
                }
                _ => panic!("unexpected capnp error return: {}", e)
            }
        }
    }
}

fn cat<R: io::Read, W: io::Write>(mut from: &mut R, to: &mut W) -> bool {
    with_entries(&mut from, move |from, ref entry| {
        assert_eq!(entry.len, copy_upto(from, to, entry.len)?);
        Ok(())
    })
}

fn grep<R: io::Read>(mut from: &mut R, regex: &regex::Regex) -> bool {
    with_entries(&mut from, move |mut from, entry| {
        let paths = join_backwards(&entry.paths, "/ /");

        for line in io::BufReader::new((&mut from).take(entry.len)).lines() {
            match line {
                Ok(line) => {
                    if regex.is_match(line.as_str()) {
                        println!("{}:{}", paths, line);
                    }
                }
                Err(e) => {
                    write!(io::stderr(), "skipping non-utf-8 ({}) file: {}\n", e, paths)?;
                }
            }
        }

        Ok(())
    })
}

fn direct_run<R: io::Read>(mut from: &mut R, cmd: &Vec<&str>) -> bool {
    with_entries(&mut from, move |mut from, entry| {
        // skip others; assuming they're empty
        if !entry.normal_file {
            assert_eq!(0, entry.len);
            return Ok(());
        }

        if !entry.content_follows {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof,
                                      "Can't do anything for contentless streams"));
        }

        let mut child = process::Command::new(cmd[0])
            .args(&cmd[1..])
            .env("TAR_REALNAME", join_backwards(&entry.paths, "/ /"))
            .env("TAR_FILENAME", entry.paths[0].to_string())
            .env("TAR_SIZE", format!("{}", entry.len))
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::inherit())
            .stderr(process::Stdio::inherit())
            .spawn()?;

        assert_eq!(entry.len, copy_upto(&mut from, &mut child.stdin.as_mut().unwrap(), entry.len)?);

        assert!(child.wait()?.success());

        Ok(())
    })
}

fn copy_upto<R: ?Sized, W: ?Sized>(reader: &mut R, writer: &mut W, how_much: u64) -> io::Result<u64>
    where R: io::Read, W: io::Write
{
    let mut buf = [0; 8 * 1024];
    let mut written = 0;
    loop {
        let max_to_read = std::cmp::min(how_much - written, buf.len() as u64) as usize;
        let len = match reader.read(&mut buf[0..max_to_read]) {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..len])?;
        written += len as u64;
    }
}

fn join_backwards(what: &Vec<String>, join: &str) -> String {
    let mut ret = String::with_capacity(what.len() * 40);

    for i in (1..(what.len() - 1)).rev() {
        ret.push_str(what[i].as_str());
        ret.push_str(join);
    }
    ret.push_str(what[0].as_str());
    ret
}


fn real_main() -> u8 {
    let from = io::stdin();
    let mut from = from.lock();

    match App::new("contentin")
        .setting(clap::AppSettings::SubcommandRequiredElseHelp)
        .subcommand(SubCommand::with_name("cat")
        )
        .subcommand(SubCommand::with_name("grep")
            .arg(Arg::with_name("pattern")
                .required(true)
                .help("pattern to search for"))
        )
        .subcommand(SubCommand::with_name("run")
                        .setting(clap::AppSettings::TrailingVarArg)
                        .setting(clap::AppSettings::DontDelimitTrailingValues)
                        .arg(Arg::with_name("sh")
                            .long("sh")
                            .help("Run with `sh -c`, and ssh-like quoting behaviour"))
//                      .arg(Arg::with_name("command-failure")
//                          .long("command-failure")
//                          .takes_value(true)
//                          .use_delimiter(false)
//                          .default_value("fatal")
//                          .possible_values(&[
//                              "fatal",
//                              "ignore",
//                          ])
                        .arg(Arg::with_name("command")
                            .required(true)
                            .help("Command to run, and its arguments")
                            .multiple(true))
        )
        .get_matches().subcommand() {
        ("cat", Some(_)) => {
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            if !cat(&mut from, &mut stdout) {
                return 2;
            }
        }
        ("grep", Some(matches)) => {
            let pattern = matches.value_of("pattern").unwrap();
            match regex::Regex::new(pattern) {
                Ok(regex) => {
                    if !grep(&mut from, &regex) {
                        return 2;
                    }
                }
                Err(e) => {
                    println!("invalid regex: {} {}", pattern, e);
                    return 2;
                }
            }
        }
        ("run", Some(matches)) => {
            let raw_command: Vec<&str> = matches.values_of("command").unwrap().collect();
            let as_dumb_line = raw_command.join(" ");

            let cmd = if matches.is_present("sh") {
                vec!("sh", "-c", as_dumb_line.as_str())
            } else {
                raw_command
            };

            if !direct_run(&mut from, &cmd) {
                return 2;
            }
        },
        _ => unreachable!(),
    }

    return 0;
}

fn main() {
    std::process::exit(real_main() as i32);
}