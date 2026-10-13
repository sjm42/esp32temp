// wifi.rs

use embedded_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration,
};
use esp_idf_svc::{
    eventloop::{EspEventLoop, System},
    ipv4,
    netif::{self, EspNetif},
    timer::{EspTimerService, Task},
    wifi::{AsyncWifi, EspWifi, WifiDriver, WifiEvent},
};

use crate::*;

pub struct WifiLoop {
    pub state: Arc<std::pin::Pin<Box<MyState>>>,
}

impl WifiLoop {
    pub async fn run(
        self,
        wifidriver: WifiDriver<'_>,
        sysloop: EspEventLoop<System>,
        timer: EspTimerService<Task>,
    ) -> anyhow::Result<()> {
        info!("Initializing Wi-Fi...");

        let event_state = self.state.clone();
        let _disconnect_sub = sysloop.subscribe::<WifiEvent, _>(move |event| {
            if let WifiEvent::StaDisconnected(d) = event {
                event_state.wifi_generation.fetch_add(1, Ordering::Relaxed);
                warn!("WiFi disconnected: reason={} rssi={}", d.reason(), d.rssi());
            }
        })?;

        let net_if = if self.state.ap_mode {
            EspNetif::new_with_conf(&netif::NetifConfiguration::wifi_default_client())?
        } else {
            let ipv4_config = if self.state.config.v4dhcp {
                ipv4::ClientConfiguration::DHCP(ipv4::DHCPClientSettings {
                    hostname: Some(
                        "esp32temp"
                            .try_into()
                            .map_err(|e| anyhow::anyhow!("Invalid hostname: {e:?}"))?,
                    ),
                })
            } else {
                ipv4::ClientConfiguration::Fixed(ipv4::ClientSettings {
                    ip: self.state.config.v4addr,
                    subnet: ipv4::Subnet {
                        gateway: self.state.config.v4gw,
                        mask: ipv4::Mask(self.state.config.v4mask),
                    },
                    dns: Some(self.state.config.dns1),
                    secondary_dns: Some(self.state.config.dns2),
                })
            };

            EspNetif::new_with_conf(&netif::NetifConfiguration {
                ip_configuration: Some(ipv4::Configuration::Client(ipv4_config)),
                ..netif::NetifConfiguration::wifi_default_client()
            })?
        };
        let mac = net_if.get_mac()?;
        *self.state.myid.write().await = format!(
            "esp32temp-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        );
        *self.state.my_mac_s.write().await = format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        );

        let ap_netif = EspNetif::new_with_conf(&netif::NetifConfiguration {
            ip_configuration: Some(ipv4::Configuration::Router(ipv4::RouterConfiguration {
                subnet: ipv4::Subnet {
                    gateway: AP_MODE_IP_ADDR,
                    mask: ipv4::Mask(AP_MODE_IP_MASK),
                },
                dhcp_enabled: true,
                dns: Some(AP_MODE_IP_ADDR),
                secondary_dns: None,
            })),
            ..netif::NetifConfiguration::wifi_default_router()
        })?;

        let espwifi = EspWifi::wrap_all(wifidriver, net_if, ap_netif)?;
        let mut wifi = AsyncWifi::wrap(espwifi, sysloop, timer)?;

        if self.state.ap_mode {
            return Box::pin(self.run_ap_mode(&mut wifi)).await;
        }

        Box::pin(self.configure(&mut wifi)).await?;

        if let Err(e) = Box::pin(self.do_connect_loop(&mut wifi, true)).await {
            error!("WiFi connection failed: {e:?}");
            error!("Resetting...");
            sleep(Duration::from_secs(5)).await;
            esp_idf_hal::reset::restart();
        }

        Box::pin(self.do_connect_loop(&mut wifi, false)).await
    }

    async fn run_ap_mode(&self, wifi: &mut AsyncWifi<EspWifi<'_>>) -> anyhow::Result<()> {
        Box::pin(self.configure_ap(wifi)).await?;

        let netif = wifi.wifi().ap_netif();
        let ip_info = netif.get_ip_info()?;
        *self.state.if_index.write().await = netif.get_index();
        *self.state.ip_addr.write().await = ip_info.ip;
        *self.state.ping_ip.write().await = None;
        *self.state.wifi_up.write().await = true;
        self.state.led_on().await?;

        info!(
            "WiFi AP mode is up at {} for manual configuration.",
            ip_info.ip
        );

        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    async fn configure_ap(&self, wifi: &mut AsyncWifi<EspWifi<'_>>) -> anyhow::Result<()> {
        info!("WiFi starting access point mode...");
        let ap_cfg = AccessPointConfiguration {
            ssid: AP_MODE_SSID
                .try_into()
                .map_err(|e| anyhow::anyhow!("Invalid AP SSID: {e:?}"))?,
            auth_method: AuthMethod::None,
            max_connections: 4,
            ..Default::default()
        };

        wifi.set_configuration(&Configuration::AccessPoint(ap_cfg))?;

        info!("WiFi driver starting...");
        Box::pin(wifi.start()).await?;

        info!("WiFi AP ready.");
        Ok(())
    }

    async fn configure(&self, wifi: &mut AsyncWifi<EspWifi<'_>>) -> anyhow::Result<()> {
        info!("WiFi setting credentials...");
        let config = &self.state.config;
        let mut client_cfg = ClientConfiguration {
            ssid: config
                .wifi_ssid
                .as_str()
                .try_into()
                .map_err(|e| anyhow::anyhow!("Invalid WiFi SSID: {e:?}"))?,
            ..Default::default()
        };
        if config.wifi_wpa2ent {
            client_cfg.auth_method = AuthMethod::WPA2Enterprise;
        } else if config.wifi_pass.is_empty() {
            client_cfg.auth_method = AuthMethod::None;
        } else {
            client_cfg.auth_method = AuthMethod::WPA2Personal;
            client_cfg.password = config
                .wifi_pass
                .as_str()
                .try_into()
                .map_err(|e| anyhow::anyhow!("Invalid WiFi password: {e:?}"))?;
        }
        if config.wifi_wpa2ent {
            let username = config.wifi_username.as_bytes();
            let password = config.wifi_pass.as_bytes();
            let username_len = i32::try_from(username.len())?;
            let password_len = i32::try_from(password.len())?;
            unsafe {
                esp_idf_sys::esp_eap_client_clear_ca_cert();
                esp_idf_sys::esp_eap_client_clear_certificate_and_key();
                esp_idf_sys::esp_eap_client_clear_identity();
                esp_idf_sys::esp_eap_client_clear_username();
                esp_idf_sys::esp_eap_client_clear_password();
                esp_idf_sys::esp_eap_client_clear_new_password();
                esp_idf_sys::esp!(esp_idf_sys::esp_eap_client_set_identity(
                    username.as_ptr(),
                    username_len
                ))?;
                esp_idf_sys::esp!(esp_idf_sys::esp_eap_client_set_username(
                    username.as_ptr(),
                    username_len
                ))?;
                esp_idf_sys::esp!(esp_idf_sys::esp_eap_client_set_password(
                    password.as_ptr(),
                    password_len
                ))?;
                esp_idf_sys::esp!(esp_idf_sys::esp_wifi_sta_enterprise_enable())?;
            }
            info!("WiFi WPA2 Enterprise credentials configured");
        }
        wifi.set_configuration(&Configuration::Client(client_cfg))?;

        // Match the sibling project's PMF capability for WPA2/WPA3 mixed-mode APs.
        unsafe {
            let mut cfg: esp_idf_sys::wifi_config_t = core::mem::zeroed();
            esp_idf_sys::esp!(esp_idf_sys::esp_wifi_get_config(
                esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
                &mut cfg
            ))?;
            cfg.sta.pmf_cfg.capable = true;
            esp_idf_sys::esp!(esp_idf_sys::esp_wifi_set_config(
                esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
                &mut cfg
            ))?;
        }

        info!("WiFi driver starting...");
        Box::pin(wifi.start()).await?;

        info!("WiFi disabling modem power save...");
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_wifi_set_ps(esp_idf_sys::wifi_ps_type_t_WIFI_PS_NONE)
        })?;

        Ok(())
    }

    async fn do_connect_loop(
        &self,
        wifi: &mut AsyncWifi<EspWifi<'_>>,
        initial: bool,
    ) -> anyhow::Result<()> {
        loop {
            // Link events wake this wait promptly on disconnection (before IP-loss timeout).
            Box::pin(wifi.wifi_wait(|w| w.is_connected(), None)).await?;
            *self.state.wifi_up.write().await = false;
            *self.state.ping_ip.write().await = None;
            *self.state.ip_addr.write().await = net::Ipv4Addr::UNSPECIFIED;
            *self.state.if_index.write().await = 0;

            info!("WiFi connecting...");
            let connected = Box::pin(async {
                Box::pin(wifi.connect()).await?;
                info!("WiFi waiting for IP...");
                Box::pin(
                    wifi.ip_wait_while(|w| w.is_up().map(|up| !up), Some(Duration::from_secs(30))),
                )
                .await
            })
            .await;
            if let Err(e) = connected {
                warn!("WiFi connection failed: {e:?}");
                if initial {
                    return Err(e.into());
                }
                // A DHCP timeout can leave the station associated; reset it before retrying.
                Box::pin(wifi.disconnect()).await?;
                sleep(Duration::from_secs(5)).await;
                continue;
            }

            let netif = wifi.wifi().sta_netif();
            let ip_info = netif.get_ip_info()?;
            *self.state.if_index.write().await = netif.get_index();
            *self.state.ip_addr.write().await = ip_info.ip;
            *self.state.ping_ip.write().await = Some(ip_info.subnet.gateway);
            self.state.wifi_generation.fetch_add(1, Ordering::Relaxed);
            *self.state.wifi_up.write().await = true;
            info!(
                "WiFi connected: {} (gateway {})",
                ip_info.ip, ip_info.subnet.gateway
            );
            if initial {
                return Ok(());
            }
        }
    }
}
// EOF
