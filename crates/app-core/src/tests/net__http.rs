use super::*;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const PARTIAL: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\na";

async fn endpoint() -> (TcpListener, reqwest::Client, NetRequest) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let client = crate::net::client_builder().no_proxy().build().unwrap();
    (listener, client, NetRequest::get(Host::Kyomu, url))
}

// A real socket lets the peer withhold the end of a generic HTTP body and
// observe its connection closing when the client abandons the response.
async fn reply(listener: &TcpListener, bytes: &[u8]) -> TcpStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut input = BufReader::new(&mut stream);
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(input.read_line(&mut line).await.unwrap(), 0);
        if line == "\r\n" {
            break;
        }
    }
    stream.write_all(bytes).await.unwrap();
    stream
}

/// Every way a response can go wrong ends the exchange in an error, never a
/// partial body, and lets go of its connection so the next read gets a
/// fresh one: a peer that stalls before the headers or inside the body, a
/// read its caller drops, a body that trickles past the absolute deadline,
/// a declared length over the limit, and an early end of the stream.
#[tokio::test]
async fn a_broken_response_ends_in_an_error_and_frees_its_connection() {
    let (listener, client, request) = endpoint().await;
    const FINISHED: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

    // Stall before the headers, stall within the body, cancel within the body.
    for (prefix, cancel) in [(b"".as_slice(), false), (PARTIAL, false), (PARTIAL, true)] {
        let mut read = Box::pin(exchange(&client, request.clone()));
        let mut peer = tokio::select! {
            peer = timeout(IO_TIMEOUT, reply(&listener, prefix)) => peer.unwrap(),
            result = &mut read => panic!("read ended while the peer was responding: {result:?}"),
        };
        // Let real socket I/O progress before advancing the stall timer.
        assert!(timeout(Duration::from_millis(25), &mut read).await.is_err());
        if cancel {
            drop(read);
        } else {
            tokio::time::pause();
            let result = timeout(request.limits.deadline + IO_TIMEOUT, read).await;
            tokio::time::resume();
            assert!(matches!(result, Ok(Err(NetError::Stalled))), "{result:?}");
        }
        let mut byte = [0; 1];
        match timeout(IO_TIMEOUT, peer.read(&mut byte)).await {
            Ok(Ok(0)) => {}
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            result => panic!("partial response was not disconnected: {result:?}"),
        }
    }

    // A body that keeps trickling in, each byte inside the stall window.
    let mut trickling = request.clone();
    trickling.limits.deadline = Duration::from_secs(1);
    let mut read = Box::pin(exchange(&client, trickling));
    let mut peer = tokio::select! {
        peer = timeout(IO_TIMEOUT, reply(&listener,
            b"HTTP/1.1 200 OK\r\nContent-Length: 2048\r\nConnection: close\r\n\r\na")) => peer.unwrap(),
        result = &mut read => panic!("read finished before the response started: {result:?}"),
    };
    let mut sent = 1;
    let result = timeout(IO_TIMEOUT, async {
        loop {
            tokio::select! {
                biased;
                result = &mut read => break result,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    peer.write_all(b"x").await.unwrap();
                    sent += 1;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(
        sent > 1,
        "the peer must make progress inside the stall window"
    );
    assert!(matches!(result, Err(NetError::Deadline)), "{result:?}");

    // Hold the socket open. The declared length alone must reject this body.
    let mut limited = request.clone();
    limited.limits.max_bytes = 1;
    let (oversized, _) = timeout(IO_TIMEOUT, async {
        tokio::join!(
            exchange(&client, limited.clone()),
            reply(&listener, PARTIAL)
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(oversized, Err(NetError::TooLarge)),
        "{oversized:?}"
    );
    limited.limits.max_bytes = 2;
    let (truncated, ()) = timeout(IO_TIMEOUT, async {
        tokio::join!(exchange(&client, limited), async {
            reply(&listener, PARTIAL).await.shutdown().await.unwrap();
        })
    })
    .await
    .unwrap();
    assert!(
        matches!(truncated, Err(NetError::Connect(_))),
        "{truncated:?}"
    );

    let (response, _) = timeout(IO_TIMEOUT, async {
        tokio::join!(exchange(&client, request), reply(&listener, FINISHED))
    })
    .await
    .unwrap();
    assert_eq!(response.unwrap().body.as_ref(), b"ok");
}
