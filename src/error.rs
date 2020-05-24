use crate::merge_futures::WrongLayoutError;
use rand;
use std::borrow::Cow;
use std::fmt;
use std::io;
use std::net;
use std::time::SystemTimeError;

#[derive(Debug)]
pub struct Error {
    pub repr: Box<ErrorRepr>,
}

impl Error {
    pub fn new<S: Into<Cow<'static, str>>>(s: S) -> Self {
        Self {
            repr: Box::new(ErrorRepr::Str(s.into())),
        }
    }
}

#[derive(Debug)]
pub enum ErrorRepr {
    Str(Cow<'static, str>),
    Io(io::Error),
    AddrParse(net::AddrParseError),
    SystemTime(SystemTimeError),
    Rand(rand::Error),
    WrongLayoutError(WrongLayoutError),
}

impl std::error::Error for Error {}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.repr {
            ErrorRepr::Str(e) => fmt::Display::fmt(e, f),
            ErrorRepr::Io(e) => fmt::Display::fmt(e, f),
            ErrorRepr::AddrParse(e) => fmt::Display::fmt(e, f),
            ErrorRepr::SystemTime(e) => fmt::Display::fmt(e, f),
            ErrorRepr::Rand(e) => fmt::Display::fmt(e, f),
            ErrorRepr::WrongLayoutError(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self {
            repr: Box::new(ErrorRepr::Io(e)),
        }
    }
}
impl From<net::AddrParseError> for Error {
    fn from(e: net::AddrParseError) -> Self {
        Self {
            repr: Box::new(ErrorRepr::AddrParse(e)),
        }
    }
}
impl From<SystemTimeError> for Error {
    fn from(e: SystemTimeError) -> Self {
        Self {
            repr: Box::new(ErrorRepr::SystemTime(e)),
        }
    }
}
impl From<rand::Error> for Error {
    fn from(e: rand::Error) -> Self {
        Self {
            repr: Box::new(ErrorRepr::Rand(e)),
        }
    }
}
impl From<WrongLayoutError> for Error {
    fn from(e: WrongLayoutError) -> Self {
        Self {
            repr: Box::new(ErrorRepr::WrongLayoutError(e)),
        }
    }
}
