// mqtt.rs

use esp_idf_svc::mqtt;
use log::*;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::*;

#[allow(unreachable_code)]
pub async fn mqtt_sender(state: Arc<Pin<Box<MyState>>>, myname: String) -> anyhow::Result<()> {
    if !state.config.read().await.mqtt_enable {
        info!("MQTT is disabled.");
        // we cannot return, otherwise tokio::select in main() will exit
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    loop {
        sleep(Duration::from_secs(10)).await;
        {
            info!("MQTT connecting...");
            let (client, conn) = match mqtt::client::EspAsyncMqttClient::new(
                &state.config.read().await.mqtt_url,
                &mqtt::client::MqttClientConfiguration {
                    client_id: Some(&myname),
                    keep_alive_interval: Some(Duration::from_secs(25)),
                    ..Default::default()
                },
            ) {
                Ok(c) => c,
                Err(e) => {
                    error!("MQTT connection failed: {e:?}");
                    continue;
                }
            };

            info!("MQTT connected.");
            tokio::join!(
                Box::pin(event_loop(conn)),
                Box::pin(data_sender(state.clone(), client))
            );
        }
    }
    Ok(())
}

async fn event_loop(mut conn: mqtt::client::EspAsyncMqttConnection) {
    while let Ok(notification) = Box::pin(conn.next()).await {
        info!("MQTT received: {:?}", notification.payload());
    }
    error!("MQTT connection closed.");
}

async fn data_sender(
    state: Arc<Pin<Box<MyState>>>,
    mut client: mqtt::client::EspAsyncMqttClient,
) -> ! {
    let mqtt_delay = state.config.read().await.mqtt_delay;
    let mqtt_topic = state.config.read().await.mqtt_topic.clone();

    loop {
        sleep(Duration::from_secs(mqtt_delay)).await;
        {
            let data = state.data.read().await;
            for v in data.temperatures.iter().filter(|v| v.value > -1000.0) {
                info!("MQTT sending {mqtt_topic}");
                if let Err(e) = client
                    .publish(
                        &mqtt_topic,
                        mqtt::client::QoS::AtLeastOnce,
                        false,
                        serde_json::to_string(v)
                            .unwrap_or_else(|_| "{}".to_string())
                            .as_bytes(),
                    )
                    .await
                {
                    error!("MQTT send error: {e}");
                }
            }
        }
    }
}

// EOF
