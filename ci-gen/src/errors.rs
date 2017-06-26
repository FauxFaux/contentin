use std;
use std::io;
use zip;

error_chain! {
    links {
        Ext4(::ext4::Error, ::ext4::ErrorKind);
    }

    foreign_links {
        Io(::std::io::Error);
        Zip(::zip::result::ZipError);
    }

    errors {
        Rewind {
            description("bailin' out")
            display("rewind")
        }
        UnsupportedFeature(msg: String) {
            description("format is (probably) legal, but we refuse to support its feature")
            display("unsupported feature: {}", msg)
        }
        Tar(msg: String) {
            description("tar-rs returned Other")
            display("tar failure message: {}", msg)
        }
    }
}

pub enum FormatErrorType {
    Rewind,
    Other,
}

pub fn is_format_error_result<T>(res: &Result<T>) -> Result<Option<FormatErrorType>> {
    if res.is_ok() {
        return Ok(None);
    }

    let error = res.as_ref().err().unwrap();

    // UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE
    // UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE
    // UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE
    // UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE
    // UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE UNSAFE

    // TODO: working around https://github.com/rust-lang/rust/issues/35943
    // TODO: i.e. error.cause() is totally useless

    let broken_ref = error.iter().last().unwrap();

    Ok(unsafe {
        let oh_look_fixed: &'static std::error::Error = std::mem::transmute(broken_ref);

        if let Some(e) = oh_look_fixed.downcast_ref::<Error>() {
            is_format_error(e)
        } else if oh_look_fixed.is::<zip::result::ZipError>() {
            // Most zip errors should be wrapped in an errors::Error,
            // but https://github.com/brson/error-chain/issues/159

            // This is just a copy-paste of is_format_error's Zip(_) => Other
            Some(FormatErrorType::Other)
        } else {
//            self.log(1, || format!("unexpectedly failed to match an error type: {:?}", broken_ref))?;
            None
        }
    })
}

fn is_format_error(e: &Error) -> Option<FormatErrorType> {
    match *e.kind() {
        ErrorKind::Rewind => {
            return Some(FormatErrorType::Rewind);
        }
        ErrorKind::Tar(_) | ErrorKind::UnsupportedFeature(_) => {
            return Some(FormatErrorType::Other);
        }
        ErrorKind::Io(ref e) => {
            // if there's an actual error code (regardless of what it is),
            // it's probably not from a library
            if e.raw_os_error().is_some() {
                return None;
            }

            match e.kind() {
                io::ErrorKind::InvalidData
                | io::ErrorKind::InvalidInput
                | io::ErrorKind::Other
                | io::ErrorKind::UnexpectedEof
                => return Some(FormatErrorType::Other),
                io::ErrorKind::BrokenPipe
                | io::ErrorKind::NotFound
                | io::ErrorKind::PermissionDenied
                => return None,
                _ => {}
            }
        }

        ErrorKind::Ext4(_) => {
            return Some(FormatErrorType::Other);
        }

        ErrorKind::Zip(_) => {
            return Some(FormatErrorType::Other);
        }

        ErrorKind::Msg(_) => {
            return None;
        }
    }

    None
}
