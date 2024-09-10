// mqtt.rs

use anyhow::bail;
use esp_idf_svc::mqtt;
use log::*;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::*;

#[allow(unreachable_code)]
pub async fn run_mqtt(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    if !state.config.read().await.mqtt_enable {
        info!("MQTT is disabled.");
        // we cannot return, otherwise tokio::select in main() will exit
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    loop {
        if *state.wifi_up.read().await {
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }

    let url = state.config.read().await.mqtt_url.clone();
    let myid = state.myid.read().await.clone();
    loop {
        sleep(Duration::from_secs(10)).await;
        {
            info!("MQTT conn: {url} [{myid}]");

            let (client, conn) = match mqtt::client::EspAsyncMqttClient::new(
                &url,
                &mqtt::client::MqttClientConfiguration {
                    client_id: Some(&myid),
                    keep_alive_interval: Some(Duration::from_secs(25)),
                    ..Default::default()
                },
            ) {
                Ok(c) => c,
                Err(e) => {
                    error!("MQTT conn failed: {e:?}");
                    continue;
                }
            };

            let _ = tokio::try_join!(
                Box::pin(data_sender(state.clone(), client)),
                Box::pin(event_loop(state.clone(), conn)),
            );
        }
    }
    Ok(())
}

async fn data_sender(
    state: Arc<Pin<Box<MyState>>>,
    mut client: mqtt::client::EspAsyncMqttClient,
) -> anyhow::Result<()> {
    let mqtt_topic = state.config.read().await.mqtt_topic.clone();

    loop {
        sleep(Duration::from_secs(5)).await;
        {
            let mut fresh_data = state.data_updated.write().await;
            if !*fresh_data { continue; }
            *fresh_data = false;
        }

        {
            let data = state.data.read().await;
            for v in data.temperatures.iter().filter(|v| v.value > NO_TEMP) {
                let topic = format!("{mqtt_topic}/{}", v.sensor);
                info!("MQTT sending {topic}");
                if let Err(e) = client
                    .publish(
                        &topic,
                        mqtt::client::QoS::AtLeastOnce,
                        false,
                        format!("{{ \"temperature\": {} }}", v.value).as_bytes(),
                    )
                    .await
                {
                    let msg = format!("MQTT send error: {e}");
                    error!("{msg}");
                    bail!("{msg}");
                }
            }
        }
    }
}

async fn event_loop(
    _state: Arc<Pin<Box<MyState>>>,
    mut conn: mqtt::client::EspAsyncMqttConnection,
) -> anyhow::Result<()> {
    while let Ok(notification) = Box::pin(conn.next()).await {
        info!("MQTT received: {:?}", notification.payload());
    }
    error!("MQTT connection closed.");
    bail!("MQTT closed.")
}

// EOF
