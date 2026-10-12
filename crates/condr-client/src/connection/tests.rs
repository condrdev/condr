use super::*;

/// A Server that takes `Hello` and never answers, as a socket's read timeout reports it.
struct Silent;

impl Read for Silent {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::WouldBlock.into())
    }
}

impl Write for Silent {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ServerStream for Silent {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self)
    }

    fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }

    fn shutdown(&self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_server_that_never_answers_did_not_answer() {
    let Err(error) = ClientConnection::welcome(Silent, "phone") else {
        panic!("a silent Server welcomed the client");
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.to_string(), "the device did not answer");
}
