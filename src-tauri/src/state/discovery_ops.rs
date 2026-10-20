use super::*;

impl AppState {
    pub async fn discover_network_receivers(
        &self,
        local_address: &str,
    ) -> Result<Vec<Candidate>, String> {
        let interface = self
            .network_interfaces()?
            .into_iter()
            .find(|item| item.address.to_string() == local_address)
            .ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
        let cached = self.last_receiver_address();
        let bindings = self.discovery_bindings();
        let cancellation = CancellationToken::new();
        let targets = discovery::probe_targets(&interface);
        let found = tokio::task::spawn_blocking(move || {
            discovery::discover_on_interface(
                &interface,
                &targets,
                cached,
                &bindings,
                Duration::from_secs(2),
                &cancellation,
            )
        })
        .await
        .map_err(|_| "网络发现任务已停止。".to_string())??;
        let now = Instant::now();
        let unix = unix_seconds();
        let mut presence = self.presence.lock().expect("presence lock");
        for candidate in &found {
            if let (Some(binding), SocketAddr::V4(endpoint)) =
                (&candidate.binding_reference, candidate.endpoint)
            {
                presence.observe(binding, *endpoint.ip(), now, unix);
            }
        }
        drop(presence);
        Ok(found)
    }

    pub(crate) fn record_presence(&self, peer: &str, binding_reference: Option<&str>) {
        let (Some(binding), Ok(SocketAddr::V4(endpoint))) =
            (binding_reference, peer.parse::<SocketAddr>())
        else {
            return;
        };
        self.presence.lock().expect("presence lock").observe(
            binding,
            *endpoint.ip(),
            Instant::now(),
            unix_seconds(),
        );
    }

    /// Probes bindings whose backoff has elapsed. Skipped while a session or an
    /// operation is active, so presence work can never compete with playback.
    pub(crate) async fn probe_presence(&self) {
        if self.session.lock().await.is_some() {
            return;
        }
        {
            let view = self.view.lock().expect("app view lock");
            if view.busy
                || view.session_active
                || selected_connection_transport(&view.settings.preferred_transport) != "wireless"
                || view.settings.wireless_protocol != "udp"
            {
                return;
            }
        }
        let bindings = self.discovery_bindings();
        if bindings.is_empty() {
            return;
        }
        let cached_default = self.last_receiver_address();
        let now = Instant::now();
        let due: Vec<String> = {
            let mut presence = self.presence.lock().expect("presence lock");
            for binding in &bindings {
                presence.register(&binding.reference, cached_default, now);
            }
            presence
                .due(now)
                .into_iter()
                .map(|(reference, _)| reference)
                .collect()
        };
        if due.is_empty() {
            return;
        }
        let Some(interface) = self.preferred_interface() else {
            return;
        };
        let due_bindings: Vec<discovery::DiscoveryBinding> = bindings
            .into_iter()
            .filter(|binding| due.iter().any(|reference| reference == &binding.reference))
            .collect();
        let targets = discovery::probe_targets(&interface);
        let cancellation = CancellationToken::new();
        let outcome = tokio::task::spawn_blocking(move || {
            discovery::discover_on_interface(
                &interface,
                &targets,
                cached_default,
                &due_bindings,
                Duration::from_millis(900),
                &cancellation,
            )
        })
        .await;
        let now = Instant::now();
        let unix = unix_seconds();
        let mut presence = self.presence.lock().expect("presence lock");
        let candidates = outcome.ok().and_then(Result::ok).unwrap_or_default();
        for reference in &due {
            match candidates.iter().find(|candidate| {
                candidate.binding_reference.as_deref() == Some(reference.as_str())
            }) {
                Some(candidate) => {
                    if let SocketAddr::V4(endpoint) = candidate.endpoint {
                        presence.observe(reference, *endpoint.ip(), now, unix);
                    }
                }
                None => presence.miss(reference, now),
            }
        }
    }

    /// First private interface with a default route; the presence prober needs one
    /// without asking the user, and a routed interface is the only useful guess.
    pub(crate) fn preferred_interface(&self) -> Option<Ipv4Interface> {
        self.network_interfaces().ok()?.into_iter().find(|item| {
            item.address.is_private() && !item.address.is_link_local() && !item.gateways.is_empty()
        })
    }

    /// Binding secrets used to authenticate discovery replies. Unreadable bindings are
    /// skipped: they simply cannot produce an authenticated candidate.
    pub(crate) fn discovery_bindings(&self) -> Vec<discovery::DiscoveryBinding> {
        self.receiver_binding_references()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|reference| {
                let path = binding_store_path(&self.data_dir, &reference).ok()?;
                let material = crate::security::BindingStore::load(&path).ok()?;
                Some(discovery::DiscoveryBinding {
                    reference,
                    secret: material.discovery_secret(),
                })
            })
            .collect()
    }

    /// Address of the Receiver that last accepted a session, if it is a usable IPv4 literal.
    pub(crate) fn last_receiver_address(&self) -> Option<Ipv4Addr> {
        let view = self.view.lock().expect("app view lock");
        let endpoint = view.settings.last_receiver_endpoint.as_deref()?;
        match endpoint.parse::<SocketAddr>().ok()?.ip() {
            IpAddr::V4(address) if !address.is_unspecified() && !address.is_loopback() => {
                Some(address)
            }
            _ => None,
        }
    }

    pub(crate) fn remember_receiver_endpoint(&self, peer: &str) {
        let Ok(endpoint) = peer.parse::<SocketAddr>() else {
            return;
        };
        if endpoint.port() == 0 {
            return;
        }
        let settings = {
            let mut view = self.view.lock().expect("app view lock");
            if view.settings.last_receiver_endpoint.as_deref() == Some(peer) {
                return;
            }
            view.settings.last_receiver_endpoint = Some(peer.to_owned());
            view.settings.clone()
        };
        let _ = self.save_settings(&settings);
    }

    #[cfg(windows)]
    pub async fn query_background_trust(
        &self,
        target: BackgroundTrustTarget,
    ) -> Result<BackgroundTrustStatus, String> {
        self.background_trust_operation(target, false, None).await
    }

    #[cfg(not(windows))]
    pub async fn query_background_trust(
        &self,
        _target: BackgroundTrustTarget,
    ) -> Result<BackgroundTrustStatus, String> {
        Err("Windows 网络 Receiver 背景授权管理仅在 Windows 上可用。".into())
    }

    #[cfg(windows)]
    pub async fn revoke_background_trust(
        &self,
        app: &AppHandle,
        target: BackgroundTrustTarget,
    ) -> Result<BackgroundTrustStatus, String> {
        self.background_trust_operation(target, true, Some(app))
            .await
    }

    #[cfg(not(windows))]
    pub async fn revoke_background_trust(
        &self,
        _app: &AppHandle,
        _target: BackgroundTrustTarget,
    ) -> Result<BackgroundTrustStatus, String> {
        Err("Windows 网络 Receiver 背景授权管理仅在 Windows 上可用。".into())
    }

    #[cfg(windows)]
    pub(crate) async fn background_trust_operation(
        &self,
        target: BackgroundTrustTarget,
        revoke: bool,
        app: Option<&AppHandle>,
    ) -> Result<BackgroundTrustStatus, String> {
        const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
        let (operation_generation, cancellation) =
            self.start_background_operation().ok_or_else(|| {
                "AnPod 正在处理其他操作；请稍后再管理 Receiver 背景授权。".to_string()
            })?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let deadline_tokio = tokio::time::Instant::from_std(deadline);
        let gate = tokio::select! {
            _ = cancellation.cancelled() => {
                cancellation.cancel();
                self.finish_operation(operation_generation);
                return Err("背景授权管理已取消。".into());
            }
            _ = tokio::time::sleep_until(deadline_tokio) => {
                cancellation.cancel();
                self.finish_operation(operation_generation);
                return Err("背景授权管理等待超时。".into());
            }
            guard = self.gate.lock() => guard,
        };
        let result = async {
            token_check(&cancellation)?;
            match target {
                BackgroundTrustTarget::ActiveSession { binding_reference, session_generation } => {
                    if !valid_binding_reference(&binding_reference) {
                        return Err("Receiver 绑定引用格式无效。".into());
                    }
                    let mut session_slot = self.session.lock().await;
                    let active = session_slot.as_mut().ok_or_else(|| "活动网络会话已结束；请刷新目标后重试。".to_string())?;
                    let active_binding = active.network_binding_ref.as_deref()
                        .ok_or_else(|| "当前会话不是可管理的 LAN Receiver 会话。".to_string())?;
                    if active_binding != binding_reference {
                        return Err("活动会话绑定与所选 Receiver 不匹配。".into());
                    }
                    if active.generation != session_generation {
                        return Err("活动会话代际已变化；请刷新授权状态后重试。".into());
                    }
                    let owner = active.network_owner.as_ref()
                        .ok_or_else(|| "活动网络会话 Owner 不可用。".to_string())?;
                    let owner_snapshot = owner.snapshot();
                    if !matches!(owner_snapshot.state, SessionLifecycleState::Authenticated
                        | SessionLifecycleState::TransportConnected | SessionLifecycleState::ReceiverRunning
                        | SessionLifecycleState::Streaming | SessionLifecycleState::Paused) {
                        return Err("Receiver 当前未处于可管理的认证会话；请等待恢复后重试。".into());
                    }
                    let trust = if revoke {
                        owner.revoke_background_trust(owner_snapshot.generation, deadline, cancellation.clone()).await?
                    } else {
                        owner.query_background_trust(owner_snapshot.generation, deadline, cancellation.clone()).await?
                    };
                    let status = background_trust_status(trust, "activeSession", binding_reference,
                        Some(session_generation))?;
                    drop(session_slot);
                    if revoke {
                        if let Some(mut ended_session) = self.session.lock().await.take() {
                            if let Some(mut ended_owner) = ended_session.network_owner.take() { ended_owner.stop(); }
                        }
                        self.finish_revoked_audio(app.expect("revoke request carries AppHandle")).await?;
                    }
                    Ok(status)
                }
                BackgroundTrustTarget::Receiver { binding_reference, local_address, peer } => {
                    if !valid_binding_reference(&binding_reference) {
                        return Err("Receiver 绑定引用格式无效。".into());
                    }
                    if self.session.lock().await.is_some() {
                        return Err("存在活动会话；请指定其 bindingReference 和 sessionGeneration。".into());
                    }
                    let interface = self.network_interfaces()?.into_iter()
                        .find(|item| item.address.to_string() == local_address)
                        .ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
                    let peer = peer.parse::<SocketAddr>()
                        .map_err(|_| "Receiver 目标必须是明确的 IPv4 地址和端口。".to_string())?;
                    let SocketAddr::V4(peer_v4) = peer else {
                        return Err("背景授权管理仅支持 IPv4 Receiver 目标。".into());
                    };
                    if peer_v4.port() == 0 || !interface.contains(*peer_v4.ip()) {
                        return Err("Receiver 目标不在所选 IPv4 接口的本地网段内。".into());
                    }
                    let path = binding_store_path(&self.data_dir, &binding_reference)?;
                    // The reference names this DPAPI file; the phone's commitment is a hash of the
                    // material, so comparing the two always failed. The management handshake below
                    // still authenticates against the phone's stored material.
                    let material = crate::security::BindingStore::load(&path)
                        .map_err(|_| "本机 DPAPI 绑定材料不可用；请重新选择已保存的 Receiver 绑定。".to_string())?;
                    let interface = interface.clone();
                    let request_cancellation = cancellation.clone();
                    let absolute_deadline = deadline;
                    let parameters = crate::security::AudioParameters { sample_rate: 48_000, channels: 2, mode: 0 };
                    let peer = SocketAddr::V4(peer_v4);
                    let host_task = tokio::task::spawn_blocking(move || {
                        let mut host = HostSession::authenticate_management_on_interface(&interface, peer,
                            material, parameters, request_cancellation.clone())?;
                        if request_cancellation.is_cancelled() || Instant::now() >= absolute_deadline {
                            return Err("Receiver 背景授权管理已取消或超时。".into());
                        }
                        if revoke {
                            host.revoke_background_trust_until(absolute_deadline, &request_cancellation)
                        } else {
                            host.query_background_trust_until(absolute_deadline, &request_cancellation)
                        }
                    });
                    let trust = tokio::select! {
                        _ = cancellation.cancelled() => {
                            cancellation.cancel();
                            return Err("背景授权管理已取消。".into());
                        }
                        _ = tokio::time::sleep_until(deadline_tokio) => {
                            cancellation.cancel();
                            return Err("Receiver 背景授权管理超时。".into());
                        }
                        joined = host_task => joined.map_err(|_| "Receiver 背景授权管理任务已停止。".to_string())??,
                    };
                    background_trust_status(trust, "receiver", binding_reference, None)
                }
            }
        }.await;
        drop(gate);
        cancellation.cancel();
        self.finish_operation(operation_generation);
        if let Some(app) = app {
            self.emit(app);
        }
        result
    }

    #[cfg(windows)]
    pub(crate) async fn finish_revoked_audio(&self, app: &AppHandle) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self.audio.stop_pcm_sink() {
            errors.push(format!("停止 PCM sink 失败：{error}"));
        }
        if let Err(error) = self.audio.stop_and_cleanup() {
            errors.push(format!("清理本机音频失败：{error}"));
        }
        if let Err(error) = self.audio.restore_muted() {
            errors.push(format!("恢复电脑输出失败：{error}"));
        }
        let mut view = self.view.lock().expect("view lock");
        view.session_active = false;
        view.session_generation = None;
        view.session_serial = None;
        view.session_receiver_binding = None;
        view.session_transport = None;
        view.session_capabilities = None;
        view.playback_diagnostics = None;
        view.network_recovery_seconds_remaining = None;
        view.audio_bitrate_bps = None;
        view.output = "computer".into();
        if errors.is_empty() {
            view.state = if view.devices.is_empty() {
                "noDevice"
            } else {
                "ready"
            }
            .into();
            view.message = "Receiver 背景授权已撤销".into();
            view.status_detail = "Receiver 已确认撤销；PCM sink 已停止，电脑输出已恢复。".into();
        } else {
            view.state = "error".into();
            view.message = "授权已撤销，但电脑音频清理未完成".into();
            view.status_detail = errors.join("；");
        }
        view.busy = false;
        drop(view);
        *self.pcm_rate_sample.lock().expect("PCM rate sample lock") = None;
        self.emit(app);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Receiver 已确认撤销，但本机清理失败：{}",
                errors.join("；")
            ))
        }
    }

    pub fn set_wireless_connection_offers(
        &self,
        local_address: &str,
        allowed: bool,
        selected_binding: Option<&str>,
    ) -> Result<HostDiscoverySnapshot, String> {
        if !allowed {
            self.host_offer_listener
                .lock()
                .expect("offer listener lock")
                .take();
            return Ok(HostDiscoverySnapshot::default());
        }
        let (interface, settings, active, busy) = {
            let view = self.view.lock().expect("app view lock");
            (
                self.network_interfaces()?
                    .into_iter()
                    .find(|item| item.address.to_string() == local_address),
                view.settings.clone(),
                view.session_active,
                view.busy,
            )
        };
        if !settings.preferred_transport.eq_ignore_ascii_case("wifi")
            || settings.wireless_protocol != "udp"
        {
            return Err("请先将无线连接方式明确设为 UDP，再允许 Receiver 发起连接请求。".into());
        }
        if active {
            return Err("活动会话期间不能更改无线连接请求监听。".into());
        }
        if busy {
            return Err("AnPod 正在处理连接操作；请稍后再更改连接请求监听。".into());
        }
        if self.operation.lock().expect("operation lock").is_some() {
            return Err("连接操作尚未结束；请稍后再允许手机连接。".into());
        }
        let references = self.receiver_binding_references()?;
        let binding = automatic_reception_binding(&references, selected_binding)?;
        let interface =
            interface.ok_or_else(|| "所选 IPv4 接口已不可用；请重新读取接口列表。".to_string())?;
        let mut listener = self
            .host_offer_listener
            .lock()
            .expect("offer listener lock");
        if let Some(current) = listener.as_ref() {
            let snapshot = current.snapshot();
            if snapshot.active
                && snapshot.interface_address == Some(interface.address)
                && snapshot.interface_prefix_length == Some(interface.prefix_length)
            {
                listener
                    .as_mut()
                    .expect("current listener")
                    .automatic_binding = binding;
                return Ok(snapshot);
            }
            listener.take();
        }
        *listener = Some(HostOfferListener::start(&interface)?);
        listener
            .as_mut()
            .expect("listener was just installed")
            .automatic_binding = binding;
        Ok(listener
            .as_ref()
            .expect("listener was just installed")
            .snapshot())
    }

    pub fn wireless_connection_offers(&self) -> HostDiscoverySnapshot {
        let Some((Some(address), Some(prefix))) = self
            .host_offer_listener
            .lock()
            .expect("offer listener lock")
            .as_ref()
            .map(|listener| {
                (
                    listener.interface_address(),
                    listener.snapshot().interface_prefix_length,
                )
            })
        else {
            return HostDiscoverySnapshot::default();
        };
        let active_interface_valid = match self.network_interfaces() {
            Ok(interfaces) => interfaces
                .iter()
                .any(|interface| interface.address == address && interface.prefix_length == prefix),
            Err(_) => false,
        };
        if !active_interface_valid {
            self.host_offer_listener
                .lock()
                .expect("offer listener lock")
                .take();
            return HostDiscoverySnapshot::default();
        }
        self.host_offer_listener
            .lock()
            .expect("offer listener lock")
            .as_ref()
            .map(HostOfferListener::snapshot)
            .unwrap_or_default()
    }

    #[cfg(windows)]
    pub async fn accept_wireless_connection_offer(
        &self,
        app: &AppHandle,
        offer_id: &str,
        binding_ref: &str,
    ) -> Result<AppSnapshot, String> {
        if self.usb_direct_configuration_active() {
            return Err("USB Direct 配置期间不能接受无线连接请求。".into());
        }
        if !valid_binding_reference(binding_ref) {
            return Err("必须明确选择已保存的 Receiver 绑定。".into());
        }
        {
            let view = self.view.lock().expect("app view lock");
            if !view
                .settings
                .preferred_transport
                .eq_ignore_ascii_case("wifi")
                || view.settings.wireless_protocol != "udp"
            {
                return Err("无线连接方式已更改；请先明确选择 UDP 并重新允许连接。".into());
            }
            if view.session_active {
                return Err("已有活动会话；请先断开后再接受连接请求。".into());
            }
            if view.busy {
                return Err("AnPod 正在处理连接操作；请稍后再接受连接请求。".into());
            }
        }
        let Some(reservation) = self.start_background_operation() else {
            return Err("AnPod 正在处理连接操作；请稍后再接受连接请求。".into());
        };
        if self.usb_direct_configuration_active() {
            self.finish_operation(reservation.0);
            return Err("USB Direct 配置期间不能接受无线连接请求。".into());
        }
        let offer_result = (|| {
            let listener = self
                .host_offer_listener
                .lock()
                .expect("offer listener lock");
            let listener = listener
                .as_ref()
                .ok_or_else(|| "无线连接请求监听已关闭。".to_string())?;
            let snapshot = listener.snapshot();
            let address = snapshot
                .interface_address
                .ok_or_else(|| "监听接口已不可用；请重新选择网络接口。".to_string())?;
            if !self.network_interfaces()?.iter().any(|item| {
                item.address == address
                    && Some(item.prefix_length) == snapshot.interface_prefix_length
            }) {
                return Err("监听接口已变化；旧连接请求已失效，请重新选择接口并允许连接。".into());
            }
            let offer = listener
                .take_offer(offer_id)
                .ok_or_else(|| "连接请求已过期或已处理，请让 Receiver 重新搜索。".to_string())?;
            Ok((offer, address, listener.cancellation()))
        })();
        let (offer, address, cancellation) = match offer_result {
            Ok(offer) => offer,
            Err(error) => {
                self.finish_operation(reservation.0);
                return Err(error);
            }
        };
        self.auto_connect_suppressed_after_aoa_switch
            .store(false, Ordering::Release);
        self.start_network_audio_inner(
            app,
            &address.to_string(),
            &offer.endpoint.to_string(),
            None,
            None,
            None,
            Some(binding_ref),
            Some((offer.nonce, false, cancellation)),
            Some(reservation),
            false,
        )
        .await
    }
}
