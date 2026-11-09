use super::*;

impl AppState {
    pub(super) fn start_qr_operation(&self) -> (u64, CancellationToken) {
        let mut current = self
            .qr_operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = current.as_ref() {
            previous.token.cancel();
        }
        let generation = self.qr_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let token = CancellationToken::new();
        *current = Some(ActiveQrOperation {
            generation,
            token: token.clone(),
        });
        (generation, token)
    }

    pub(super) fn finish_qr_operation(&self, generation: u64) {
        let mut current = self
            .qr_operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            *current = None;
        }
    }

    pub async fn connect_wifi(&self, endpoint: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if self
            .last_pairing_endpoint
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
            == Some(endpoint.as_str())
        {
            return Err(
                "这是配对端口，不能用于连接；请搜索设备或使用手机无线调试主页上的连接端口。".into(),
            );
        }
        let result = checked_adb(
            &self.adb_path,
            None,
            &["connect", &endpoint],
            Duration::from_secs(10),
            None,
        )
        .await;
        let success = result.is_ok();
        let message = result
            .err()
            .unwrap_or_else(|| format!("已连接到 {endpoint}。"));
        if success {
            self.persist_wireless_endpoint(&endpoint)?;
            let _ = self.refresh_devices().await;
        }
        Ok(OperationResult {
            success,
            message,
            snapshot: self.snapshot(),
        })
    }

    pub async fn pair_wifi(&self, endpoint: &str, code: &str) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let endpoint = normalize_endpoint(endpoint)?;
        if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
            return Err("配对码应为 6 位数字。".into());
        }
        let result = adb_run(
            &self.adb_path,
            None,
            &["pair", &endpoint, code],
            Duration::from_secs(20),
            None,
        )
        .await?;
        let success = result.success
            && result
                .stdout
                .to_ascii_lowercase()
                .contains("successfully paired");
        let mut message = if success {
            "配对成功。正在查找连接端口…".into()
        } else {
            "配对失败。请确认配对端口和 6 位配对码仍有效。".into()
        };
        if success {
            *self
                .last_pairing_endpoint
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(endpoint.clone());
            if let Ok(discovery) = self.discover_wifi().await {
                let host = endpoint
                    .rsplit_once(':')
                    .map(|(host, _)| host)
                    .unwrap_or("");
                let matches: Vec<_> = discovery["services"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|service| {
                        service["kind"] == "connect"
                            && service["endpoint"]
                                .as_str()
                                .unwrap_or("")
                                .starts_with(&format!("{host}:"))
                    })
                    .collect();
                if matches.len() == 1 {
                    if let Some(connect_endpoint) = matches[0]["endpoint"].as_str() {
                        if checked_adb(
                            &self.adb_path,
                            None,
                            &["connect", connect_endpoint],
                            Duration::from_secs(10),
                            None,
                        )
                        .await
                        .is_ok()
                        {
                            self.persist_wireless_endpoint(connect_endpoint)?;
                            let _ = self.refresh_devices().await;
                            message = "配对成功，并已连接手机。".into();
                        }
                    }
                } else if matches.len() > 1 {
                    message = "配对成功，但发现多个连接端口；请确认后手动选择。".into();
                } else {
                    message = "配对成功；未发现连接端口，请检查手机无线调试是否仍开启。".into();
                }
            } else {
                message = "配对成功；mDNS 搜索不可用，请手动输入无线调试连接端口。".into();
            }
        }
        if success {
            crate::applog::info("pairing", "wireless.succeeded", "");
        } else {
            crate::applog::warn("pairing", "wireless.failed", "");
        }
        Ok(OperationResult {
            success,
            message,
            snapshot: self.snapshot(),
        })
    }

    pub(super) fn persist_wireless_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if view.session_active {
            return Ok(());
        }
        let mut settings = view.settings.clone();
        settings.last_wireless_endpoint = Some(endpoint.to_owned());
        self.save_settings(&settings)?;
        view.settings = settings;
        Ok(())
    }

    pub async fn discover_wifi(&self) -> Result<Value, String> {
        self.discover_wifi_with_cancel(None).await
    }

    pub(super) async fn discover_wifi_with_cancel(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, String> {
        let check = adb_run(
            &self.adb_path,
            None,
            &["mdns", "check"],
            Duration::from_secs(5),
            cancel,
        )
        .await?;
        let combined = format!("{}\n{}", check.stdout, check.stderr).to_ascii_lowercase();
        if combined.contains("unavailable")
            || combined.contains("not running")
            || !combined.contains("mdns daemon version")
        {
            return Err("ADB mDNS 搜索不可用；请手动填写手机的 IP:连接端口。".into());
        }
        let services = adb_run(
            &self.adb_path,
            None,
            &["mdns", "services"],
            Duration::from_secs(6),
            cancel,
        )
        .await?;
        if !services.success {
            return Err("读取 ADB mDNS 设备失败。".into());
        }
        let found: Vec<Value> = services.stdout.lines().filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let endpoint = fields.iter().rev().find(|item| normalize_endpoint(item).is_ok())?;
            let line_l = line.to_ascii_lowercase();
            let kind = if line_l.contains("_adb-tls-pairing") { "pairing" } else if line_l.contains("_adb-tls-connect") || line_l.contains("_adb._tcp") { "connect" } else { return None; };
            Some(serde_json::json!({"name": fields.first().copied().unwrap_or(*endpoint), "endpoint": endpoint, "kind": kind}))
        }).collect();
        Ok(serde_json::json!({"services": found, "available": true}))
    }

    pub fn qr_credentials(&self) -> Result<Value, String> {
        use getrandom::fill;
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
        let mut bytes = [0u8; 30];
        fill(&mut bytes).map_err(|_| "无法生成配对凭据。".to_string())?;
        let instance: String = bytes[..10]
            .iter()
            .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
            .collect();
        let password: String = bytes[10..]
            .iter()
            .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
            .collect();
        let name = format!("studio-{instance}");
        let payload = format!("WIFI:T:ADB;S:{name};P:{password};;");
        Ok(serde_json::json!({"instanceName":name,"password":password,"payload":payload}))
    }

    pub async fn pair_qr(
        &self,
        app: &AppHandle,
        instance: String,
        password: String,
    ) -> Result<OperationResult, String> {
        if instance.len() != 17
            || !instance.starts_with("studio-")
            || !instance
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || password.len() != 20
            || !password.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("二维码配对凭据格式无效。".into());
        }
        let (generation, token) = self.start_qr_operation();
        let result = self
            .pair_qr_with_token(app, &instance, &password, &token)
            .await;
        self.finish_qr_operation(generation);
        result
    }

    pub(super) async fn pair_qr_with_token(
        &self,
        app: &AppHandle,
        instance: &str,
        password: &str,
        token: &CancellationToken,
    ) -> Result<OperationResult, String> {
        let _guard = self.gate.lock().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let endpoint = loop {
            token_check(token)?;
            let found = self.discover_wifi_with_cancel(Some(token)).await?;
            let services = found
                .get("services")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(pairing) = services.into_iter().find(|s| {
                s["kind"] == "pairing"
                    && normalize_mdns_instance(s["name"].as_str().unwrap_or("")) == instance
            }) {
                break pairing["endpoint"]
                    .as_str()
                    .ok_or("未收到配对端口")?
                    .to_owned();
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("等待扫码超时，请重新生成二维码。".into());
            }
            let _ = app.emit("anpod://pairing-progress", "等待手机扫码…");
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(800)) => {} }
        };
        *self
            .last_pairing_endpoint
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(endpoint.clone());
        let _ = app.emit("anpod://pairing-progress", "已识别手机，正在配对…");
        let pair = adb_run(
            &self.adb_path,
            None,
            &["pair", &endpoint, password],
            Duration::from_secs(20),
            Some(token),
        )
        .await?;
        if !pair.success
            || !pair
                .stdout
                .to_ascii_lowercase()
                .contains("successfully paired")
        {
            return Err("二维码配对失败，请重新生成后重试。".into());
        }
        crate::applog::info("pairing", "wireless_qr.succeeded", "");
        let host = endpoint.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let (connect, connect_name) = loop {
            token_check(token)?;
            let services = self.discover_wifi_with_cancel(Some(token)).await?;
            let found = services["services"].as_array().cloned().unwrap_or_default();
            let matches: Vec<_> = found
                .iter()
                .filter(|service| {
                    service["kind"] == "connect"
                        && service["endpoint"]
                            .as_str()
                            .is_some_and(|value| value.starts_with(&format!("{host}:")))
                })
                .collect();
            if matches.len() == 1 {
                break (
                    matches[0]["endpoint"]
                        .as_str()
                        .ok_or("缺少二维码手机的无线连接端口")?
                        .to_owned(),
                    matches[0]["name"]
                        .as_str()
                        .ok_or("缺少二维码手机的连接实例名")?
                        .to_owned(),
                );
            }
            if matches.len() > 1 {
                return Err("二维码手机广播了多个连接端口；请手动选择无线调试连接地址。".into());
            }
            if tokio::time::Instant::now() >= connect_deadline {
                return Err(
                    "手机已配对，但没有发现对应的无线连接端口。请确认无线调试仍开启后重试。".into(),
                );
            }
            let _ = app.emit(
                "anpod://pairing-progress",
                "配对完成，正在等待手机广播连接端口…",
            );
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(700)) => {} }
        };
        checked_adb(
            &self.adb_path,
            None,
            &["connect", &connect],
            Duration::from_secs(10),
            Some(token),
        )
        .await?;
        let device_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        let device = loop {
            token_check(token)?;
            let snapshot = self.refresh_devices_with_cancel(Some(token)).await?;
            if let Some(device) = snapshot.devices.into_iter().find(|device| {
                device.transport == "wifi"
                    && device.state == "device"
                    && device.connectable
                    && (device.serial == connect
                        || normalize_mdns_instance(&device.serial)
                            == normalize_mdns_instance(&connect_name))
            }) {
                break device;
            }
            if tokio::time::Instant::now() >= device_deadline {
                return Err("无线连接命令已完成，但 ADB 未显示这部手机为已授权设备。请在设备列表中检查连接状态。".into());
            }
            tokio::select! { _ = token.cancelled() => return Err("二维码配对已取消。".into()), _ = tokio::time::sleep(Duration::from_millis(500)) => {} }
        };
        token_check(token)?;
        self.persist_wireless_endpoint(&connect)?;
        let mut view = self
            .view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !view.session_active {
            view.selected_serial = Some(device.serial);
        }
        drop(view);
        Ok(OperationResult {
            success: true,
            message: "配对并连接完成。".into(),
            snapshot: self.snapshot(),
        })
    }

    pub fn cancel_qr_pairing(&self) {
        if let Some(active) = self
            .qr_operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            active.token.cancel();
        }
    }
}
