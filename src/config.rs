// config.rs

use anyhow::{anyhow, bail};
use embedded_storage::{ReadStorage, Storage};
use esp_storage::FlashStorage;
use log::*;
use serde::{Deserialize, Serialize};

/*
   Boot message example, where we can see 0x6000 = 6x4 KiB = 24 KiB
   partition for data starting at 0x9000

    Hence, we are using the last 1 KiB of that range here.

I (42) boot: ESP-IDF v5.1-beta1-378-gea5e0ff298-dirt 2nd stage bootloader
I (42) boot: compile time Jun  7 2023 07:59:10
I (43) boot: chip revision: v0.4
I (47) boot.esp32c3: SPI Speed      : 40MHz
I (52) boot.esp32c3: SPI Mode       : DIO
I (57) boot.esp32c3: SPI Flash Size : 4MB
I (62) boot: Enabling RNG early entropy source...
I (67) boot: Partition Table:
I (71) boot: ## Label            Usage          Type ST Offset   Length
I (78) boot:  0 nvs              WiFi data        01 02 00009000 00006000
I (85) boot:  1 phy_init         RF data          01 01 0000f000 00001000
I (93) boot:  2 factory          factory app      00 00 00010000 003f0000
I (100) boot: End of partition table
I (105) esp_image: segment 0: paddr=00010020 vaddr=3c100020 size=500a8h (327848) map
I (185) esp_image: segment 1: paddr=000600d0 vaddr=3fc8fc00 size=032cch ( 13004) load
I (188) esp_image: segment 2: paddr=000633a4 vaddr=40380000 size=0cc74h ( 52340) load
I (204) esp_image: segment 3: paddr=00070020 vaddr=42000020 size=fa830h (1026096) map
I (428) esp_image: segment 4: paddr=0016a858 vaddr=4038cc74 size=02df8h ( 11768) load
I (437) boot: Loaded app from partition at offset 0x10000
*/

pub const FLASH_NVS_ADDR: u32 = 0xEC00;
pub const FLASH_BUF_SIZE: usize = 0x400; // more than enough

const DEFAULT_BOOT_FAIL_MAX: u8 = 4;
const DEFAULT_API_PORT: u16 = 80;
const DEFAULT_SENSOR_RETRIES: u32 = 4;
const DEFAULT_POLL_DELAY: u64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyConfig {
    pub boot_fail_cnt: u8,
    pub boot_fail_max: u8,

    pub api_port: u16,
    pub sensor_retries: u32,
    pub poll_delay: u64,

    pub wifi_ssid: String,
    pub wifi_pass: String,
}

impl Default for MyConfig {
    fn default() -> Self {
        Self {
            boot_fail_cnt: 0,
            boot_fail_max: DEFAULT_BOOT_FAIL_MAX,

            api_port: option_env!("API_PORT")
                .unwrap_or("-")
                .parse()
                .unwrap_or(DEFAULT_API_PORT),
            sensor_retries: DEFAULT_SENSOR_RETRIES,
            poll_delay: DEFAULT_POLL_DELAY,

            wifi_ssid: option_env!("WIFI_SSID").unwrap_or("internet").into(),
            wifi_pass: option_env!("WIFI_PASS").unwrap_or("password").into(),
        }
    }
}

impl MyConfig {
    pub fn from_flash(flash: &mut FlashStorage) -> Option<Self> {
        // with Box we are using heap instead of stack here
        let mut flashbuf = Box::new([0u8; FLASH_BUF_SIZE]);
        info!("Reading {sz} bytes from flash...", sz = FLASH_BUF_SIZE);
        if let Err(e) = flash.read(FLASH_NVS_ADDR, &mut *flashbuf) {
            error!("Flash read error {e:?}");
            return None;
        }

        let lenbuf = [flashbuf[0], flashbuf[1]];
        let config_len = u16::from_be_bytes(lenbuf) as usize;
        if !(4..=FLASH_BUF_SIZE - 2).contains(&config_len) {
            error!("Flash config size is invalid");
            return None;
        }
        info!("Parsing {config_len} bytes of config");
        let config_raw = &flashbuf[2..config_len + 2];
        let config_str = match std::str::from_utf8(config_raw) {
            Ok(s) => s,
            Err(e) => {
                error!("Flash config UTF8 error: {e}");
                return None;
            }
        };

        match serde_json::from_str::<MyConfig>(config_str) {
            Err(e) => {
                error!("Flash config JSON error: {e}");
                error!("Offending JSON:\n{config_str}");
                None
            }
            Ok(c) => {
                info!("Successfully parsed config from flash.");
                Some(c)
            }
        }
    }

    pub fn to_flash(&self, flash: &mut FlashStorage) -> anyhow::Result<()> {
        // with Box we are using heap instead of stack here
        let mut flashbuf = Box::new([0u8; FLASH_BUF_SIZE]);
        let jconfig = serde_json::to_string(self)?;
        let sz = jconfig.len();

        if sz > FLASH_BUF_SIZE - 2 {
            bail!("Config too big! Size={sz}");
        }
        info!("Saving {sz} bytes of config to flash. Config:\n{jconfig}");

        flashbuf[0..2].copy_from_slice(&(sz as u16).to_be_bytes());
        flashbuf[2..2 + sz].copy_from_slice(jconfig.as_bytes());
        match flash.write(FLASH_NVS_ADDR, &*flashbuf) {
            Err(e) => {
                let estr = format!("Flash write error: {e:?}");
                error!("{estr}");
                Err(anyhow!(estr))
            }
            Ok(_) => Ok(()),
        }
    }
}

// EOF
