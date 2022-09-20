use std;
use std::io;

use anyhow::Result;

#[derive(Debug, thiserror::Error)]
pub enum ErrorKind {
    #[error("bailin' out")]
    Rewind,

    /// format is (probably) legal, but we refuse to support its feature
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),
}

#[derive(Debug, PartialEq)]
pub enum FormatErrorType {
    Rewind,
    Other,
}

pub fn classify_format_error_result<T>(res: &Result<T>) -> Option<FormatErrorType> {
    let error = match res {
        Ok(_) => return None,
        Err(err) => err,
    };

    let root_cause = error.root_cause();

    if let Some(e) = root_cause.downcast_ref::<ErrorKind>() {
        is_format_error(e)
    } else if root_cause.is::<zip::result::ZipError>() {
        Some(FormatErrorType::Other)
    } else if root_cause.is::<ext4::ParseError>() {
        Some(FormatErrorType::Other)
    } else if let Some(e) = root_cause.downcast_ref::<io::Error>() {
        is_io_format_error(e).unwrap_or(None)
    } else {
        //            self.log(1, || format!("unexpectedly failed to match an error type: {:?}", broken_ref))?;
        None
    }
}

fn is_format_error(e: &ErrorKind) -> Option<FormatErrorType> {
    match e {
        ErrorKind::Rewind => {
            return Some(FormatErrorType::Rewind);
        }
        ErrorKind::UnsupportedFeature(_) => {
            return Some(FormatErrorType::Other);
        }
    }
}

fn is_io_format_error(e: &io::Error) -> Option<Option<FormatErrorType>> {
    // if there's an actual error code (regardless of what it is),
    // it's probably not from a library
    if e.raw_os_error().is_some() {
        return Some(None);
    }

    match e.kind() {
        io::ErrorKind::InvalidData
        | io::ErrorKind::InvalidInput
        | io::ErrorKind::Other
        | io::ErrorKind::UnexpectedEof => return Some(Some(FormatErrorType::Other)),
        io::ErrorKind::BrokenPipe | io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            return Some(None)
        }
        _ => {}
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use std::io;
    use zip;

    fn simulate_failure(input: bool) -> zip::result::ZipResult<bool> {
        if !input {
            Ok(input)
        } else {
            Err(zip::result::ZipError::FileNotFound)
        }
    }

    fn simulate_second_failure(input: bool) -> zip::result::ZipResult<bool> {
        if !input {
            Ok(input)
        } else {
            Err(zip::result::ZipError::Io(io::ErrorKind::BrokenPipe.into()))
        }
    }

    #[test]
    fn real_format_error() {
        let failure = simulate_failure(true).with_context(|| "oops");
        assert_eq!(
            classify_format_error_result(&failure).unwrap(),
            FormatErrorType::Other
        );
    }

    #[test]
    fn io_error_is_not_format() {
        let err: io::Error = io::ErrorKind::BrokenPipe.into();
        let res: Result<()> = Err(err).with_context(|| "oops");
        assert!(classify_format_error_result(&res).is_none())
    }

    #[test]
    fn nested_zip_failure_is_not_format() {
        let failure = simulate_second_failure(true).with_context(|| "oops");
        assert!(classify_format_error_result(&failure).is_none())
    }

    #[test]
    fn chain_syntax_irrelevant() {
        let explicit = simulate_failure(true).context("whoopsie").unwrap_err();
        let exp_cause = explicit.source().unwrap();
        assert!(exp_cause.is::<zip::result::ZipError>());
    }
}
