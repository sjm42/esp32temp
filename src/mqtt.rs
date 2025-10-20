// mqtt.rs

use esp_idf_svc::mqtt::{self, client::MessageId};
use esp_idf_sys::EspError;

use crate::*;

#[allow(unreachable_code)]
pub async fn run_mqtt(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    if !state.config.mqtt_enable {
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

    let url = state.config.mqtt_url.clone();
    let myid = state.myid.read().await.clone();

    sleep(Duration::from_secs(10)).await;

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
            let emsg = format!("MQTT conn failed: {e:?}");
            error!("{emsg}");
            bail!("{emsg}");
        }
    };

    tokio::select! {
        _ = Box::pin(data_sender(state.clone(), client)) => { error!("data_sender() ended."); }
        _ = Box::pin(event_loop(state.clone(), conn)) => { error!("event_loop() ended."); }
    };
    Ok(())
}

async fn data_sender(
    state: Arc<Pin<Box<MyState>>>,
    mut client: mqtt::client::EspAsyncMqttClient,
) -> anyhow::Result<()> {
    let mqtt_topic = state.config.mqtt_topic.clone();

    loop {
        sleep(Duration::from_secs(5)).await;
        let uptime = *(state.uptime.read().await);

        {
            let mut fresh_data = state.data_updated.write().await;
            if !*fresh_data {
                continue;
            }
            *fresh_data = false;
        }

        {
            let mut topic = format!("{mqtt_topic}/uptime");
            let mut mqtt_data = format!("{{ \"uptime\": {} }}", uptime);
            Box::pin(mqtt_send(&mut client, &topic, &mqtt_data)).await?;

            let data = state.data.read().await;
            for v in data.temperatures.iter().filter(|v| v.value > NO_TEMP) {
                topic = format!("{mqtt_topic}/{}", v.sensor);
                mqtt_data = format!("{{ \"temperature\": {} }}", v.value);
                Box::pin(mqtt_send(&mut client, &topic, &mqtt_data)).await?;
            }
        }
    }
}

async fn mqtt_send(
    client: &mut mqtt::client::EspAsyncMqttClient,
    topic: &str,
    data: &str,
) -> Result<MessageId, EspError> {
    info!("MQTT sending {topic} {data}");

    let result = client
        .publish(
            topic,
            mqtt::client::QoS::AtLeastOnce,
            false,
            data.as_bytes(),
        )
        .await;
    if let Err(e) = result {
        let msg = format!("MQTT send error: {e}");
        error!("{msg}");
    }
    result
}

async fn event_loop(
    _state: Arc<Pin<Box<MyState>>>,
    mut conn: mqtt::client::EspAsyncMqttConnection,
) -> anyhow::Result<()> {
    while let Ok(notification) = Box::pin(conn.next()).await {
        info!("MQTT received: {:?}", notification.payload());
    }

    error!("MQTT connection closed.");
    Ok(())
}
// EOF
