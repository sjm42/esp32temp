// mqtt.rs

use log::*;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::*;

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub async fn mqtt_sender(state: Arc<Pin<Box<MyState>>>, myname: String) -> anyhow::Result<()> {
    if !state.config.read().await.mqtt_enable {
        info!("MQTT is disabled.");
        // we cannot return, otherwise tokio::select in main() will exit
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    // do not go any further than this for now...

    loop {
        sleep(Duration::from_secs(3600)).await;
    }

    // After this point, we will get: "memory allocation of 262144 bytes failed" and kaboom. Reset.

    let mut mqttoptions = MqttOptions::new(
        myname,
        state.config.read().await.mqtt_server.to_string(),
        state.config.read().await.mqtt_port,
    );
    mqttoptions
        .set_keep_alive(Duration::from_secs(25))
        .set_clean_session(true);
    let (client, eventloop) = rumqttc::AsyncClient::new(mqttoptions, 2);

    tokio::join!(Box::pin(eloop(eventloop)), Box::pin(sender(state, client)));
    Ok(())
}

async fn eloop(mut eventloop: EventLoop) -> ! {
    loop {
        if let Ok(notification) = Box::pin(eventloop.poll()).await {
            info!("MQTT received: {notification:?}");
        }
    }
}

async fn sender(state: Arc<Pin<Box<MyState>>>, client: AsyncClient) -> ! {
    let mqtt_delay = state.config.read().await.mqtt_delay;
    let mqtt_topic = state.config.read().await.mqtt_topic.clone();

    loop {
        sleep(Duration::from_secs(mqtt_delay)).await;
        let data = state.data.read().await;
        for v in data.temperatures.iter().filter(|v| v.value > -1000.0) {
            let topic = format!("{}/{}", mqtt_topic, v.sensor);
            info!("MQTT sending {topic}");
            if let Err(e) = client
                .publish(topic, QoS::AtLeastOnce, false, v.value.to_string())
                .await
            {
                error!("MQTT send error: {e}");
            }
        }
        drop(data);
    }
}

// EOF
