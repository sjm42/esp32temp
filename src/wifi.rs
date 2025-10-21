// wifi.rs

use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::{
    eventloop::{EspEventLoop, System},
    ipv4,
    netif::{self, EspNetif},
    timer::{EspTimerService, Task},
    wifi::{AsyncWifi, EspWifi, WifiDriver},
};

use crate::*;

pub struct WifiLoop<'a> {
    pub state: Arc<std::pin::Pin<Box<MyState>>>,
    pub wifi: Option<AsyncWifi<EspWifi<'a>>>,
}

impl<'a> WifiLoop<'a> {
    pub async fn run(
        mut self,
        wifidriver: WifiDriver<'_>,
        sysloop: EspEventLoop<System>,
        timer: EspTimerService<Task>,
    ) -> anyhow::Result<()> {
        info!("Initializing Wi-Fi...");

        let ipv4_config = if self.state.config.v4dhcp {
            ipv4::ClientConfiguration::DHCP(ipv4::DHCPClientSettings::default())
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
        // info!("IP config: {ipv4_config:?}");

        let net_if = EspNetif::new_with_conf(&netif::NetifConfiguration {
            ip_configuration: Some(ipv4::Configuration::Client(ipv4_config)),
            ..netif::NetifConfiguration::wifi_default_client()
        })?;
        let mac = net_if.get_mac()?;
        *self.state.myid.write().await = format!(
            "esp32temp-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        );

        let espwifi = EspWifi::wrap_all(wifidriver, net_if, EspNetif::new(netif::NetifStack::Ap)?)?;
        self.wifi = Some(AsyncWifi::wrap(espwifi, sysloop, timer.clone())?);
        Box::pin(self.configure()).await?;

        if let Err(e) = Box::pin(self.initial_connect()).await {
            error!("WiFi connection failed: {e:?}");
            error!("Resetting...");
            sleep(Duration::from_secs(5)).await;
            esp_idf_hal::reset::restart();
        }

        sleep(Duration::from_secs(5)).await;

        let netif = self.wifi.as_ref().unwrap().wifi().sta_netif();
        let ip_info = netif.get_ip_info()?;
        *self.state.if_index.write().await = netif.get_index();
        *self.state.ip_addr.write().await = ip_info.ip;
        *self.state.ping_ip.write().await = Some(ip_info.subnet.gateway);
        *self.state.wifi_up.write().await = true;

        Box::pin(self.stay_connected()).await
    }

    pub async fn configure(&mut self) -> anyhow::Result<()> {
        info!("WiFi setting credentials...");
        let wifi = self.wifi.as_mut().unwrap();
        let config = &self.state.config;
        let mut client_cfg = ClientConfiguration {
            ssid: config.wifi_ssid.as_str().try_into().unwrap(),
            ..Default::default()
        };
        if config.wifi_pass.is_empty() {
            client_cfg.auth_method = AuthMethod::None;
        } else {
            client_cfg.auth_method = AuthMethod::WPA2Personal;
            client_cfg.password = config.wifi_pass.as_str().try_into().unwrap();
        }
        if config.wifi_wpa2ent {
            client_cfg.auth_method = AuthMethod::WPA2Enterprise;
            let username = config.wifi_username.as_str();
            let password = config.wifi_pass.as_str();
            unsafe {
                esp_idf_sys::esp_eap_client_clear_ca_cert();
                esp_idf_sys::esp_eap_client_clear_certificate_and_key();
                esp_idf_sys::esp_eap_client_clear_identity();
                esp_idf_sys::esp_eap_client_clear_username();
                esp_idf_sys::esp_eap_client_clear_password();
                esp_idf_sys::esp_eap_client_clear_new_password();
                // let ret1 = esp_idf_sys::esp_eap_client_set_username(username.as_ptr(), username.len() as i32);
                let ret1 = esp_idf_sys::esp_eap_client_set_identity(
                    username.as_ptr(),
                    username.len() as i32,
                );
                let ret2 = esp_idf_sys::esp_eap_client_set_username(
                    username.as_ptr(),
                    username.len() as i32,
                );
                let ret3 = esp_idf_sys::esp_eap_client_set_password(
                    password.as_ptr(),
                    password.len() as i32,
                );
                // let ret4 = esp_idf_sys::esp_eap_client_set_new_password(password.as_ptr(), password.len() as i32);
                let ret4 = esp_idf_sys::esp_wifi_sta_enterprise_enable();

                info!("WiFi WPA2 Enterprise: {ret1}:{ret2}:{ret3}:{ret4}");
            }
        }
        wifi.set_configuration(&Configuration::Client(client_cfg))?;

        info!("WiFi driver starting...");
        Ok(Box::pin(wifi.start()).await?)
    }

    pub async fn initial_connect(&mut self) -> anyhow::Result<()> {
        self.do_connect_loop(true).await
    }

    pub async fn stay_connected(mut self) -> anyhow::Result<()> {
        self.do_connect_loop(false).await
    }

    async fn do_connect_loop(&mut self, initial: bool) -> anyhow::Result<()> {
        let wifi = self.wifi.as_mut().unwrap();
        loop {
            // Wait for disconnect before trying to connect again.  This loop ensures
            // we stay connected and is commonly missing from trivial examples as it's
            // way too difficult to showcase the core logic of an example and have
            // a proper Wi-Fi event loop without a robust async runtime.  Fortunately, we can do it
            // now!
            let timeout = if initial {
                Some(Duration::from_secs(30))
            } else {
                None
            };
            Box::pin(wifi.wifi_wait(|w| w.is_up(), timeout)).await.ok();

            info!("WiFi connecting...");
            Box::pin(wifi.connect()).await.ok();

            info!("WiFi waiting for association...");
            match Box::pin(wifi.ip_wait_while(|w| w.is_up().map(|s| !s), None)).await {
                Ok(_) => {}
                Err(e) => {
                    error!("WiFi error: {e:?}");

                    // only exit here if this is initial connection
                    // otherwise, keep trying
                    if initial {
                        bail!(e);
                    }
                }
            }

            info!("WiFi connected.");
            if initial {
                return Ok(());
            }
        }
    }
}

// EOF
