use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout, Instant};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub const OKX_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/business";
const HEARTBEAT_INTERVAL: u64 = 25;

pub async fn run_okx_ws(inst_id: &str, channel: &str, inst_type: &str) -> anyhow::Result<()> {
    let (ws_stream, _) = connect_async(OKX_WS_URL).await?;
    log::info!(
        "OKX websocket connected, channel: {}, inst_id: {}, inst_type: {}",
        channel,
        inst_id,
        inst_type
    );

    let (mut write, mut read) = ws_stream.split();
    let sub_msg = json!({
        "op": "subscribe",
        "args": [
            {
                "channel": channel,
                "instId": inst_id,
                "instType": inst_type
            }
        ]
    })
    .to_string();
    write.send(Message::Text(sub_msg)).await?;

    let (ping_tx, mut ping_rx) = mpsc::channel::<()>(1);
    let ping_tx_clone = ping_tx.clone();
    tokio::spawn(async move {
        let mut timer = interval(Duration::from_secs(HEARTBEAT_INTERVAL));
        loop {
            timer.tick().await;
            let _ = ping_tx_clone.send(()).await;
        }
    });

    let mut last_recv = Instant::now();
    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text.trim() == "pong" {
                            log::debug!("OKX websocket pong");
                        } else if text.contains("data") {
                            log::debug!("OKX websocket data: {}", text);
                        }
                        last_recv = Instant::now();
                    }
                    Some(Ok(Message::Binary(_))) => {
                        last_recv = Instant::now();
                    }
                    Some(Ok(Message::Ping(_))) => {
                        write.send(Message::Pong(Vec::new())).await?;
                        last_recv = Instant::now();
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_recv = Instant::now();
                    }
                    Some(Ok(Message::Close(frame))) => {
                        log::warn!("OKX websocket closed: {:?}", frame);
                        break;
                    }
                    Some(Ok(Message::Frame(_))) => {
                        last_recv = Instant::now();
                    }
                    Some(Err(e)) => {
                        log::error!("OKX websocket error: {:?}", e);
                        break;
                    }
                    None => {
                        log::warn!("OKX websocket stream ended");
                        break;
                    }
                }
            }
            _ = ping_rx.recv() => {
                if last_recv.elapsed() >= Duration::from_secs(HEARTBEAT_INTERVAL) {
                    write.send(Message::Text("ping".to_string())).await?;
                    if timeout(Duration::from_secs(HEARTBEAT_INTERVAL), async {
                        loop {
                            let msg = read.next().await;
                            if let Some(Ok(Message::Text(resp))) = msg {
                                if resp.trim() == "pong" {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }).await.is_err() {
                        log::warn!("OKX websocket ping timeout");
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}
