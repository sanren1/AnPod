use super::*;

impl SessionOwner {
    pub(crate) fn cancel_signal(&self) -> Arc<dyn Fn() + Send + Sync> {
        let enabled = self.enabled.clone();
        let cancellation = self.cancellation.clone();
        Arc::new(move || {
            enabled.store(false, Ordering::Release);
            cancellation.cancel();
        })
    }

    pub(crate) fn start(
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::start_with_timing(
            peer,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(crate) fn start_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::start_with_endpoint(
            SessionEndpoint::on_interface(interface, peer),
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_on_interface_with_profile(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        harmony: bool,
        initial_float_queue: Option<Arc<FloatPcmQueue>>,
        caller_cancel: CancellationToken,
        connection_intent: Option<([u8; 16], bool)>,
        background_authorization_required: bool,
        diagnostics: Option<StartupDiagnosticsCapture>,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.harmony = harmony;
        endpoint.connection_intent = connection_intent;
        endpoint.background_authorization_required = background_authorization_required;
        Self::start_with_endpoint_and_profile(
            endpoint,
            binding,
            parameters,
            profile,
            initial_float_queue,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
            diagnostics,
        )
    }

    pub(crate) fn start_on_interface_with_intent(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        nonce: [u8; 16],
        required: bool,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.connection_intent = Some((nonce, required));
        Self::start_with_endpoint(
            endpoint,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(crate) fn start_on_interface_with_background_authorization(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
    ) -> Result<Self, String> {
        let mut endpoint = SessionEndpoint::on_interface(interface, peer);
        endpoint.background_authorization_required = true;
        Self::start_with_endpoint(
            endpoint,
            binding,
            parameters,
            caller_cancel,
            DEFAULT_SESSION_TIMING,
        )
    }

    pub(super) fn start_with_timing(
        peer: SocketAddr,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
    ) -> Result<Self, String> {
        Self::start_with_endpoint(
            SessionEndpoint::loopback(peer),
            binding,
            parameters,
            caller_cancel,
            timing,
        )
    }

    pub(super) fn start_with_endpoint(
        endpoint: SessionEndpoint,
        binding: BindingMaterial,
        parameters: AudioParameters,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
    ) -> Result<Self, String> {
        Self::start_with_endpoint_and_profile(
            endpoint,
            binding,
            parameters,
            HostAudioProfile::Standard,
            None,
            caller_cancel,
            timing,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_with_endpoint_and_profile(
        mut endpoint: SessionEndpoint,
        binding: BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        initial_float_queue: Option<Arc<FloatPcmQueue>>,
        caller_cancel: CancellationToken,
        timing: SessionTiming,
        diagnostics: Option<StartupDiagnosticsCapture>,
    ) -> Result<Self, String> {
        endpoint.validate()?;
        if endpoint.background_request_id.is_none() {
            endpoint.background_request_id = Some(random_request_id()?);
        }
        let cancellation = caller_cancel.child_token();
        let enabled = Arc::new(AtomicBool::new(true));
        let status = Arc::new(Mutex::new(SessionSnapshot {
            state: SessionLifecycleState::Reconnecting,
            generation: 0,
            attempts: 0,
            session_id: None,
            capabilities: None,
            paused: false,
            last_error: None,
            terminal_cause: None,
            playback_diagnostics: None,
            negotiated_format: None,
            recovery_seconds_remaining: None,
            recovery_started_at: None,
            recovery_window: timing.recovery_window,
        }));
        let queue = Arc::new(Mutex::new(None));
        let float_queue = Arc::new(Mutex::new(None));
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        let (commands, receiver) = mpsc::channel();
        let owner_cancel = cancellation.clone();
        let owner_enabled = enabled.clone();
        let owner_status = status.clone();
        let owner_queue = queue.clone();
        let owner_float_queue = float_queue.clone();
        let owner_bytes = pcm_payload_bytes_sent.clone();
        let worker = thread::Builder::new()
            .name("AnPod P5-D session owner".into())
            .spawn(move || {
                run_session_owner(
                    endpoint,
                    binding,
                    parameters,
                    owner_cancel,
                    owner_enabled,
                    receiver,
                    owner_status,
                    owner_queue,
                    owner_float_queue,
                    owner_bytes,
                    timing,
                    profile,
                    initial_float_queue,
                    diagnostics,
                )
            })
            .map_err(|error| format!("Unable to start P5-D session owner: {error}"))?;
        Ok(Self {
            harmony: endpoint.harmony,
            connection_intent: endpoint.connection_intent,
            background_authorization_required: endpoint.background_authorization_required,
            cancellation,
            enabled,
            commands,
            status,
            queue,
            float_queue,
            pcm_payload_bytes_sent,
            worker: Some(worker),
        })
    }

    pub(crate) fn harmony(&self) -> bool {
        self.harmony
    }

    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        let mut snapshot = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        snapshot.recovery_seconds_remaining =
            if snapshot.state == SessionLifecycleState::Reconnecting {
                snapshot.recovery_started_at.map(|started| {
                    snapshot
                        .recovery_window
                        .saturating_sub(started.elapsed())
                        .as_secs_f64()
                        .ceil() as u64
                })
            } else {
                None
            };
        snapshot
    }

    pub(crate) async fn query_background_trust(
        &self,
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err("背景授权查询已取消或超时。".into());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(OwnerCommand::BackgroundTrustQuery {
                session_generation,
                deadline,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        tokio::select! {
            biased;
            result = response => result.map_err(|_| "P5-D session owner stopped before the background trust query completed.".to_string())?,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err("背景授权查询已超时。".into()),
        }
    }

    pub(crate) async fn revoke_background_trust(
        &self,
        session_generation: u64,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err("背景授权撤销已取消或超时。".into());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(OwnerCommand::BackgroundTrustRevoke {
                session_generation,
                deadline,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        tokio::select! {
            biased;
            result = response => result.map_err(|_| "P5-D session owner stopped before the background trust revocation completed.".to_string())?,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err("背景授权撤销已超时。".into()),
        }
    }

    pub(crate) fn pcm_queue(&self) -> Option<Arc<PcmQueue>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn audio_queue(&self) -> Option<SessionAudioQueue> {
        if let Some(queue) = self
            .float_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            let format = self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .negotiated_format?;
            Some(SessionAudioQueue::Float32(queue, format))
        } else {
            self.pcm_queue().map(SessionAudioQueue::Pcm16)
        }
    }

    pub(crate) fn matches_audio_quality(&self, quality: &str) -> bool {
        let profile = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .negotiated_format
            .map(|format| format.profile);
        matches!(
            (quality, profile),
            ("standard", None) | ("hifi", Some(4)) | ("hires", Some(5))
        )
    }

    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub(crate) fn set_paused(&self, paused: bool) -> Result<(), String> {
        self.commands
            .send(OwnerCommand::SetPaused(paused))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    #[cfg(test)]
    pub(super) fn sender_barrier_for_test(&self, timeout: Duration) -> Result<bool, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.commands
            .send(OwnerCommand::TestSenderBarrier(reply))
            .map_err(|_| "P5-D session owner has stopped.".to_string())?;
        response
            .recv_timeout(timeout)
            .map_err(|_| "P5-D sender barrier was not reached.".to_string())
    }

    pub(crate) fn set_mode(&self, mode: u8) -> Result<(), String> {
        if mode > 1 {
            return Err("P5-C playback mode is invalid.".into());
        }
        self.commands
            .send(OwnerCommand::SetMode(mode))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn reconfigure_audio(
        &self,
        profile: HostAudioProfile,
        parameters: AudioParameters,
        paused: bool,
    ) -> Result<(), String> {
        if parameters.mode > 1 || parameters.channels != 2 {
            return Err("P5-D audio configuration is invalid.".into());
        }
        let profile = profile_for_mode(profile, parameters.mode)?;
        if let HostAudioProfile::AudioV2(format) = profile {
            if format.sample_rate != parameters.sample_rate {
                return Err("P5-D audio profile does not match its sample rate.".into());
            }
        }
        self.commands
            .send(OwnerCommand::ReconfigureAudio {
                profile,
                parameters,
                paused,
            })
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn reconnect(&self) -> Result<(), String> {
        self.commands
            .send(OwnerCommand::Reconnect)
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    /// The caller must supply each changed endpoint; the owner never discovers or guesses one.
    pub(crate) fn update_endpoint(&self, peer: SocketAddr) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::loopback(peer))
    }

    pub(crate) fn update_network_endpoint(
        &self,
        interface: &Ipv4Interface,
        peer: SocketAddr,
    ) -> Result<(), String> {
        self.enqueue_endpoint(SessionEndpoint::on_interface(interface, peer))
    }

    pub(super) fn enqueue_endpoint(&self, mut endpoint: SessionEndpoint) -> Result<(), String> {
        endpoint.harmony = self.harmony;
        endpoint.connection_intent = self.connection_intent;
        endpoint.background_authorization_required = self.background_authorization_required;
        endpoint.background_request_id = Some(random_request_id()?);
        endpoint.validate()?;
        self.commands
            .send(OwnerCommand::UpdateEndpoint(endpoint))
            .map_err(|_| "P5-D session owner has stopped.".to_string())
    }

    pub(crate) fn cancel(&mut self) {
        self.enabled.store(false, Ordering::Release);
        self.cancellation.cancel();
        self.join();
    }

    pub(crate) fn stop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        if self.commands.send(OwnerCommand::Stop).is_err() {
            self.enabled.store(false, Ordering::Release);
            self.cancellation.cancel();
        } else {
            let deadline = Instant::now() + Duration::from_millis(100);
            while self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(5));
            }
            if self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
            {
                self.enabled.store(false, Ordering::Release);
                self.cancellation.cancel();
            }
        }
        self.join();
        self.enabled.store(false, Ordering::Release);
        self.cancellation.cancel();
    }

    pub(super) fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}
