// esphome_api.rs

use std::collections::BTreeMap;

use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::*;

const ESPHOME_API_PORT: u16 = 6053;
const API_VERSION_MAJOR: u32 = 1;
const API_VERSION_MINOR: u32 = 14;

const STATE_CLASS_NONE: u32 = 0;
const STATE_CLASS_MEASUREMENT: u32 = 1;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApiMessageType {
    HelloRequest = 1,
    HelloResponse = 2,
    AuthRequest = 3,
    DisconnectRequest = 5,
    DisconnectResponse = 6,
    PingRequest = 7,
    PingResponse = 8,
    DeviceInfoRequest = 9,
    DeviceInfoResponse = 10,
    ListEntitiesRequest = 11,
    ListEntitiesSensorResponse = 16,
    ListEntitiesTextSensorResponse = 18,
    ListEntitiesDoneResponse = 19,
    SubscribeStatesRequest = 20,
    SensorStateResponse = 25,
    TextSensorStateResponse = 27,
    SubscribeHomeassistantServicesRequest = 34,
    SubscribeHomeassistantStatesRequest = 38,
    NoiseEncryptionSetKeyRequest = 124,
    NoiseEncryptionSetKeyResponse = 125,
}

impl ApiMessageType {
    const fn id(self) -> u32 {
        self as u32
    }
}

impl TryFrom<u32> for ApiMessageType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::HelloRequest),
            2 => Ok(Self::HelloResponse),
            3 => Ok(Self::AuthRequest),
            5 => Ok(Self::DisconnectRequest),
            6 => Ok(Self::DisconnectResponse),
            7 => Ok(Self::PingRequest),
            8 => Ok(Self::PingResponse),
            9 => Ok(Self::DeviceInfoRequest),
            10 => Ok(Self::DeviceInfoResponse),
            11 => Ok(Self::ListEntitiesRequest),
            16 => Ok(Self::ListEntitiesSensorResponse),
            18 => Ok(Self::ListEntitiesTextSensorResponse),
            19 => Ok(Self::ListEntitiesDoneResponse),
            20 => Ok(Self::SubscribeStatesRequest),
            25 => Ok(Self::SensorStateResponse),
            27 => Ok(Self::TextSensorStateResponse),
            34 => Ok(Self::SubscribeHomeassistantServicesRequest),
            38 => Ok(Self::SubscribeHomeassistantStatesRequest),
            124 => Ok(Self::NoiseEncryptionSetKeyRequest),
            125 => Ok(Self::NoiseEncryptionSetKeyResponse),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntityKind {
    Sensor,
    TextSensor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EntitySource {
    Uptime,
    LastUpdate,
    Temperature { address_hex: String },
}

#[derive(Clone, Debug)]
struct EntityDef {
    source: EntitySource,
    key: u32,
    object_id: String,
    name: String,
    kind: EntityKind,
    unit: Option<String>,
    accuracy: i32,
    device_class: Option<String>,
    state_class: u32,
}

#[derive(Clone, Debug, PartialEq)]
enum EntityStateValue {
    Missing,
    Number(f32),
    Text(String),
}

pub async fn run_esphome_api(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    if !state.config.esphome_enable {
        info!("ESPHome API is disabled.");
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

    let listen = format!("0.0.0.0:{ESPHOME_API_PORT}");
    let addr = listen.parse::<net::SocketAddr>()?;
    let listener = TcpListener::bind(addr).await?;
    info!("ESPHome API listening on {listen}");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("ESPHome API client connected: {peer}");
        let state2 = state.clone();
        tokio::spawn(async move {
            if let Err(e) = Box::pin(handle_client(state2, stream)).await {
                warn!("ESPHome API client error: {e}");
            }
            info!("ESPHome API client disconnected: {peer}");
        });
    }
}

async fn handle_client(state: Arc<Pin<Box<MyState>>>, mut stream: TcpStream) -> anyhow::Result<()> {
    let mut state_subscribed = false;
    let mut entities = build_entity_defs(&state).await;
    let mut last_sent = BTreeMap::<u32, EntityStateValue>::new();

    loop {
        match Box::pin(timeout(Duration::from_secs(5), read_frame(&mut stream))).await {
            Ok(Ok((msg_type_raw, payload))) => match ApiMessageType::try_from(msg_type_raw) {
                Ok(ApiMessageType::HelloRequest) => {
                    if let Some((client_info, major, minor)) = parse_hello_request(&payload) {
                        info!(
                            "ESPHome hello from '{client_info}' API {major}.{minor} (server {API_VERSION_MAJOR}.{API_VERSION_MINOR})"
                        );
                    } else {
                        info!("ESPHome hello request received");
                    }
                    send_hello_response(&state, &mut stream).await?;
                }
                Ok(ApiMessageType::AuthRequest) => {
                    info!("ESPHome auth request ignored (password auth removed upstream)");
                }
                Ok(ApiMessageType::PingRequest) => {
                    send_frame(&mut stream, ApiMessageType::PingResponse, &[]).await?;
                }
                Ok(ApiMessageType::DisconnectRequest) => {
                    send_frame(&mut stream, ApiMessageType::DisconnectResponse, &[]).await?;
                    return Ok(());
                }
                Ok(ApiMessageType::DeviceInfoRequest) => {
                    send_device_info_response(&state, &mut stream).await?;
                }
                Ok(ApiMessageType::ListEntitiesRequest) => {
                    entities = build_entity_defs(&state).await;
                    send_list_entities_response(&mut stream, &entities).await?;
                }
                Ok(ApiMessageType::SubscribeStatesRequest) => {
                    state_subscribed = true;
                    Box::pin(send_state_updates(
                        &state,
                        &mut stream,
                        &entities,
                        &mut last_sent,
                        true,
                    ))
                    .await?;
                }
                Ok(ApiMessageType::SubscribeHomeassistantServicesRequest)
                | Ok(ApiMessageType::SubscribeHomeassistantStatesRequest) => continue,
                Ok(ApiMessageType::NoiseEncryptionSetKeyRequest) => {
                    let mut payload = Vec::new();
                    pb_put_bool(1, false, &mut payload);
                    send_frame(
                        &mut stream,
                        ApiMessageType::NoiseEncryptionSetKeyResponse,
                        &payload,
                    )
                    .await?;
                }
                Ok(msg_type) => {
                    debug!("ESPHome API: unhandled message type {msg_type:?}");
                    continue;
                }
                Err(_) => {
                    debug!("ESPHome API: unhandled message type {msg_type_raw}");
                    continue;
                }
            },
            Ok(Err(e)) => {
                if is_closed_connection(&e) {
                    return Ok(());
                }
                return Err(e.into());
            }
            Err(_) => {
                debug!("ESPHome API poll tick");
            }
        }

        if state_subscribed {
            Box::pin(send_state_updates(
                &state,
                &mut stream,
                &entities,
                &mut last_sent,
                false,
            ))
            .await?;
        }
    }
}

async fn send_hello_response(
    state: &Arc<Pin<Box<MyState>>>,
    stream: &mut TcpStream,
) -> anyhow::Result<()> {
    let device_name = state.myid.read().await.clone();
    let mut payload = Vec::new();
    pb_put_varint(1, API_VERSION_MAJOR, &mut payload);
    pb_put_varint(2, API_VERSION_MINOR, &mut payload);
    pb_put_string(3, &format!("esp32temp {FW_VERSION}"), &mut payload);
    pb_put_string(4, &device_name, &mut payload);
    send_frame(stream, ApiMessageType::HelloResponse, &payload).await?;
    Ok(())
}

async fn send_device_info_response(
    state: &Arc<Pin<Box<MyState>>>,
    stream: &mut TcpStream,
) -> anyhow::Result<()> {
    let mut payload = Vec::new();
    let device_name = state.myid.read().await.clone();
    let device_mac = state.my_mac_s.read().await.clone();

    pb_put_string(2, &device_name, &mut payload);
    pb_put_string(3, &device_mac, &mut payload);
    pb_put_string(4, FW_VERSION, &mut payload);
    pb_put_string(5, "", &mut payload);
    pb_put_string(6, "ESP32", &mut payload);
    pb_put_string(12, "Espressif", &mut payload);
    pb_put_string(13, "Temperature probe", &mut payload);

    send_frame(stream, ApiMessageType::DeviceInfoResponse, &payload).await?;
    Ok(())
}

async fn send_list_entities_response(
    stream: &mut TcpStream,
    entities: &[EntityDef],
) -> anyhow::Result<()> {
    for entity in entities {
        match entity.kind {
            EntityKind::Sensor => {
                let mut payload = Vec::new();
                pb_put_string(1, &entity.object_id, &mut payload);
                pb_put_fixed32(2, entity.key, &mut payload);
                pb_put_string(3, &entity.name, &mut payload);
                if let Some(unit) = &entity.unit {
                    pb_put_string(6, unit, &mut payload);
                }
                pb_put_varint(7, entity.accuracy as u32, &mut payload);
                if let Some(device_class) = &entity.device_class {
                    pb_put_string(9, device_class, &mut payload);
                }
                pb_put_varint(10, entity.state_class, &mut payload);
                send_frame(stream, ApiMessageType::ListEntitiesSensorResponse, &payload).await?;
            }
            EntityKind::TextSensor => {
                let mut payload = Vec::new();
                pb_put_string(1, &entity.object_id, &mut payload);
                pb_put_fixed32(2, entity.key, &mut payload);
                pb_put_string(3, &entity.name, &mut payload);
                if let Some(device_class) = &entity.device_class {
                    pb_put_string(8, device_class, &mut payload);
                }
                send_frame(
                    stream,
                    ApiMessageType::ListEntitiesTextSensorResponse,
                    &payload,
                )
                .await?;
            }
        }
    }

    send_frame(stream, ApiMessageType::ListEntitiesDoneResponse, &[]).await?;
    Ok(())
}

async fn send_state_updates(
    state: &Arc<Pin<Box<MyState>>>,
    stream: &mut TcpStream,
    entities: &[EntityDef],
    last_sent: &mut BTreeMap<u32, EntityStateValue>,
    force: bool,
) -> anyhow::Result<()> {
    let current_states = build_entity_states(state, entities).await;
    last_sent.retain(|key, _| current_states.contains_key(key));

    for entity in entities {
        let value = current_states
            .get(&entity.key)
            .cloned()
            .unwrap_or(EntityStateValue::Missing);
        let changed = force || last_sent.get(&entity.key) != Some(&value);
        if !changed {
            continue;
        }

        match (&entity.kind, &value) {
            (EntityKind::Sensor, EntityStateValue::Number(v)) => {
                let mut payload = Vec::new();
                pb_put_fixed32(1, entity.key, &mut payload);
                pb_put_float(2, *v, &mut payload);
                send_frame(stream, ApiMessageType::SensorStateResponse, &payload).await?;
            }
            (EntityKind::Sensor, EntityStateValue::Missing)
            | (EntityKind::Sensor, EntityStateValue::Text(_)) => {
                let mut payload = Vec::new();
                pb_put_fixed32(1, entity.key, &mut payload);
                pb_put_bool(3, true, &mut payload);
                send_frame(stream, ApiMessageType::SensorStateResponse, &payload).await?;
            }
            (EntityKind::TextSensor, EntityStateValue::Text(v)) => {
                let mut payload = Vec::new();
                pb_put_fixed32(1, entity.key, &mut payload);
                pb_put_string(2, v, &mut payload);
                send_frame(stream, ApiMessageType::TextSensorStateResponse, &payload).await?;
            }
            (EntityKind::TextSensor, EntityStateValue::Number(v)) => {
                let mut payload = Vec::new();
                pb_put_fixed32(1, entity.key, &mut payload);
                pb_put_string(2, &v.to_string(), &mut payload);
                send_frame(stream, ApiMessageType::TextSensorStateResponse, &payload).await?;
            }
            (EntityKind::TextSensor, EntityStateValue::Missing) => {
                let mut payload = Vec::new();
                pb_put_fixed32(1, entity.key, &mut payload);
                pb_put_bool(3, true, &mut payload);
                send_frame(stream, ApiMessageType::TextSensorStateResponse, &payload).await?;
            }
        }

        last_sent.insert(entity.key, value);
    }

    Ok(())
}

async fn build_entity_defs(state: &Arc<Pin<Box<MyState>>>) -> Vec<EntityDef> {
    let sensors = state.sensors.read().await;
    let mut entities = Vec::new();

    entities.push(EntityDef {
        source: EntitySource::Uptime,
        key: stable_key("uptime"),
        object_id: "uptime".into(),
        name: "Uptime".into(),
        kind: EntityKind::Sensor,
        unit: Some("s".into()),
        accuracy: 0,
        device_class: Some("duration".into()),
        state_class: STATE_CLASS_MEASUREMENT,
    });
    entities.push(EntityDef {
        source: EntitySource::LastUpdate,
        key: stable_key("last_update"),
        object_id: "last_update".into(),
        name: "Last Update".into(),
        kind: EntityKind::TextSensor,
        unit: None,
        accuracy: 0,
        device_class: None,
        state_class: STATE_CLASS_NONE,
    });

    for onewire in sensors.iter() {
        for address in onewire.ids.iter() {
            let address_hex = format!("{:016X}", address.0);
            let object_id = format!("temperature_{}", address_hex.to_ascii_lowercase());
            entities.push(EntityDef {
                source: EntitySource::Temperature {
                    address_hex: address_hex.clone(),
                },
                key: stable_key(&object_id),
                object_id,
                name: format!("Temperature {} {}", onewire.name, address_hex),
                kind: EntityKind::Sensor,
                unit: Some("\u{00B0}C".into()),
                accuracy: 2,
                device_class: Some("temperature".into()),
                state_class: STATE_CLASS_MEASUREMENT,
            });
        }
    }

    entities
}

async fn build_entity_states(
    state: &Arc<Pin<Box<MyState>>>,
    entities: &[EntityDef],
) -> BTreeMap<u32, EntityStateValue> {
    let data = state.data.read().await.clone();
    let mut temp_map = BTreeMap::new();
    for temp in data.temperatures.iter() {
        if temp.value > NO_TEMP {
            temp_map.insert(temp.sensor.clone(), temp.value);
        }
    }

    let mut out = BTreeMap::new();
    for entity in entities {
        let value = match &entity.source {
            EntitySource::Uptime => EntityStateValue::Number(data.uptime as f32),
            EntitySource::LastUpdate => {
                if data.last_update == "-" {
                    EntityStateValue::Missing
                } else {
                    EntityStateValue::Text(data.last_update.clone())
                }
            }
            EntitySource::Temperature { address_hex } => match temp_map.get(address_hex) {
                Some(value) => EntityStateValue::Number(*value),
                None => EntityStateValue::Missing,
            },
        };
        out.insert(entity.key, value);
    }

    out
}

async fn read_frame(stream: &mut TcpStream) -> io::Result<(u32, Vec<u8>)> {
    let preamble = stream.read_u8().await?;
    if preamble != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid preamble 0x{preamble:02X}"),
        ));
    }

    let payload_len = read_varuint_async(stream).await? as usize;
    let msg_type = read_varuint_async(stream).await? as u32;
    if payload_len > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("payload too large: {payload_len}"),
        ));
    }

    let mut payload = vec![0_u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload).await?;
    }
    Ok((msg_type, payload))
}

async fn send_frame(
    stream: &mut TcpStream,
    msg_type: ApiMessageType,
    payload: &[u8],
) -> io::Result<()> {
    let mut frame = Vec::with_capacity(1 + 10 + 10 + payload.len());
    frame.push(0x00);
    put_varuint(payload.len() as u64, &mut frame);
    put_varuint(u64::from(msg_type.id()), &mut frame);
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await
}

async fn read_varuint_async(stream: &mut TcpStream) -> io::Result<u64> {
    let mut result = 0_u64;
    let mut shift = 0_u32;
    for _ in 0..10 {
        let byte = stream.read_u8().await?;
        result |= (u64::from(byte & 0x7F)) << shift;
        if (byte & 0x80) == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "varuint overflow",
    ))
}

fn parse_hello_request(payload: &[u8]) -> Option<(String, u32, u32)> {
    let mut idx = 0_usize;
    let mut client_info = String::new();
    let mut major = 0_u32;
    let mut minor = 0_u32;

    while idx < payload.len() {
        let key = read_varuint_from_slice(payload, &mut idx)?;
        let field_number = (key >> 3) as u32;
        let wire_type = (key & 0x07) as u8;
        match wire_type {
            0 => {
                let value = read_varuint_from_slice(payload, &mut idx)? as u32;
                match field_number {
                    2 => major = value,
                    3 => minor = value,
                    _ => {}
                }
            }
            2 => {
                let len = read_varuint_from_slice(payload, &mut idx)? as usize;
                if idx + len > payload.len() {
                    return None;
                }
                if field_number == 1 {
                    client_info = std::str::from_utf8(&payload[idx..idx + len])
                        .ok()?
                        .to_string();
                }
                idx += len;
            }
            1 => idx += 8,
            5 => idx += 4,
            _ => return None,
        }
        if idx > payload.len() {
            return None;
        }
    }

    Some((client_info, major, minor))
}

fn read_varuint_from_slice(data: &[u8], idx: &mut usize) -> Option<u64> {
    let mut out = 0_u64;
    let mut shift = 0_u32;
    for _ in 0..10 {
        if *idx >= data.len() {
            return None;
        }
        let b = data[*idx];
        *idx += 1;
        out |= (u64::from(b & 0x7F)) << shift;
        if (b & 0x80) == 0 {
            return Some(out);
        }
        shift += 7;
    }
    None
}

fn is_closed_connection(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    )
}

fn stable_key(object_id: &str) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for b in object_id.as_bytes() {
        hash ^= u32::from(*b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash == 0 { 1 } else { hash }
}

fn put_varuint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value as u8) & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn pb_put_key(field_number: u32, wire_type: u8, out: &mut Vec<u8>) {
    put_varuint(u64::from((field_number << 3) | u32::from(wire_type)), out);
}

fn pb_put_varint(field_number: u32, value: u32, out: &mut Vec<u8>) {
    pb_put_key(field_number, 0, out);
    put_varuint(u64::from(value), out);
}

fn pb_put_bool(field_number: u32, value: bool, out: &mut Vec<u8>) {
    pb_put_key(field_number, 0, out);
    out.push(if value { 1 } else { 0 });
}

fn pb_put_fixed32(field_number: u32, value: u32, out: &mut Vec<u8>) {
    pb_put_32bit(field_number, value.to_le_bytes(), out);
}

fn pb_put_float(field_number: u32, value: f32, out: &mut Vec<u8>) {
    pb_put_32bit(field_number, value.to_le_bytes(), out);
}

fn pb_put_32bit(field_number: u32, bytes: [u8; 4], out: &mut Vec<u8>) {
    pb_put_key(field_number, 5, out);
    out.extend_from_slice(&bytes);
}

fn pb_put_string(field_number: u32, value: &str, out: &mut Vec<u8>) {
    pb_put_key(field_number, 2, out);
    put_varuint(value.len() as u64, out);
    out.extend_from_slice(value.as_bytes());
}

// EOF
