use std::fs;
use std::io;
use std::time;

use crates_time;
use ext4;
use stat;

use errors::*;

pub fn simple_time(dur: time::Duration) -> u64 {
    dur.as_secs().checked_mul(1_000_000_000).map_or(
        0,
        |nanos| {
            nanos + dur.subsec_nanos() as u64
        },
    )
}

pub fn simple_time_sys(val: time::SystemTime) -> u64 {
    val.duration_since(time::UNIX_EPOCH)
        .map(simple_time)
        .unwrap_or(0)
}


pub fn simple_time_tm(val: crates_time::Tm) -> u64 {
    let timespec = val.to_timespec();
    simple_time(time::Duration::new(
        timespec.sec as u64,
        timespec.nsec as u32,
    ))
}

pub fn simple_time_btime(val: &fs::Metadata) -> Result<u64> {
    match val.created() {
        Ok(time) => Ok(simple_time_sys(time)),
        // "Other" is how "unsupported" is represented here; ew.
        Err(ref e) if e.kind() == io::ErrorKind::Other => Ok(0),
        Err(other) => Err(other).chain_err(|| "loading btime"),
    }
}

pub fn simple_time_ext4(val: &ext4::Time) -> u64 {
    let nanos = val.nanos.unwrap_or(0);
    if nanos > 1_000_000_000 {
        // TODO: there are some extra bits here for us, which I'm too lazy to implement
        return 0;
    }

    if val.epoch_secs > 0x7fff_ffff {
        // Negative time, which we're actually not supporting?
        return 0;
    }

    (val.epoch_secs as u64) * 1_000_000_000 + nanos as u64
}

pub fn simple_time_epoch_seconds(seconds: u64) -> u64 {
    seconds.checked_mul(1_000_000_000).unwrap_or(0)
}

pub fn simple_time_ctime(val: &stat::Stat) -> u64 {
    if val.ctime <= 0 {
        0
    } else {
        (val.ctime as u64).checked_mul(1_000_000_000).unwrap_or(0) + (val.ctime_nano as u64)
    }
}