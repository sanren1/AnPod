use super::*;

impl HostSession {
    pub(crate) fn authenticate_loopback(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::authenticate_loopback_cancellable(
            peer,
            material,
            parameters,
            CancellationToken::new(),
        )
    }

    pub(crate) fn authenticate_management_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let mut session = Self::authenticate_on_network(
            SessionEndpoint::on_interface(interface, peer),
            material,
            parameters,
            cancellation,
        )?;
        let result = session.negotiate_capabilities().and_then(|_| {
            if session
                .capabilities()
                .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
            {
                Ok(())
            } else {
                Err("Receiver does not support background trust management.".into())
            }
        });
        if let Err(error) = result {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    pub(super) fn authenticate_loopback_cancellable(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        Self::authenticate_on_network(
            SessionEndpoint::loopback(peer),
            material,
            parameters,
            parent_cancel,
        )
    }

    pub(super) fn authenticate_on_network(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        parameters: AudioParameters,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        parameters
            .validate()
            .map_err(|_| "Unsupported P5-C PCM parameters.".to_string())?;
        endpoint.validate()?;
        let cancellation = parent_cancel.child_token();
        check_cancelled(&cancellation).map_err(|_| "P5-C connection was cancelled.".to_string())?;
        let udp_socket = UdpSocket::bind((endpoint.local_address, 0)).map_err(|_| {
            "Unable to bind the P5-C audio socket to the selected interface.".to_string()
        })?;
        let mut tcp = connect_cancellable(endpoint.local_address, endpoint.peer, &cancellation)
            .map_err(|error| if error.kind() == std::io::ErrorKind::AddrNotAvailable {
                "The selected network interface is no longer available.".to_string()
            } else {
                "Receiver endpoint is unavailable on the selected interface; it may be unreachable or require authorized USB/bootstrap if stopped.".to_string()
            })?;
        tcp.set_read_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let binding_commitment = material.commitment();
        let (handshake, hello) = HostHandshake::start(material, parameters)
            .map_err(|_| "P5-A handshake could not start.".to_string())?;
        write_all_cancellable(&mut tcp, &hello, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A HELLO.".to_string())?;
        let mut challenge = [0u8; 86];
        read_exact_cancellable(&mut tcp, &mut challenge, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake
            .accept_challenge(&challenge)
            .map_err(|_| "P5-A receiver identity or proof was rejected.".to_string())?;
        write_all_cancellable(&mut tcp, &finish, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A FINISH.".to_string())?;
        let mut ready = [0u8; 38];
        read_exact_cancellable(&mut tcp, &mut ready, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A READY.".to_string())?;
        let session = await_ready
            .accept_ready(&ready)
            .map_err(|_| "P5-A READY proof was rejected.".to_string())?;
        let session_id = session.session_id();
        let authenticated_parameters = session.parameters();
        let keys = session.into_keys();
        let session_keys = SessionKeys {
            audio: keys.host_to_receiver_audio,
            control_tx: keys.host_to_receiver_control,
            control_rx: keys.receiver_to_host_control,
        };
        drop(keys);
        let format = PcmFormat {
            sample_rate: authenticated_parameters.sample_rate,
            channels: authenticated_parameters.channels as u16,
        };
        let pcm_payload_bytes_sent = Arc::new(AtomicU64::new(0));
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            float_sender: None,
            pcm_payload_bytes_sent,
            queue: Arc::new(PcmQueue::for_session(
                format,
                authenticated_parameters.mode == 1,
            )),
            float_queue: None,
            keys: session_keys,
            parameters: authenticated_parameters,
            format_v2: None,
            session_id,
            binding_commitment,
            remote_udp: None,
            command_sequence: 0,
            ack_sequence: 0,
            last_command: None,
            last_ack: None,
            last_heartbeat: Instant::now(),
            state: ReceiverState::Authenticated,
            capabilities: None,
            terminal_cause: None,
            connection_intent: endpoint.connection_intent,
            background_request_id: endpoint.background_request_id,
            background_authorization_required: endpoint.background_authorization_required,
            background_authorized: false,
            cancellation,
        })
    }

    pub(super) fn authenticate_v2_on_network(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        format: AudioFormatV2,
        float_queue: Arc<FloatPcmQueue>,
        parent_cancel: CancellationToken,
    ) -> Result<Self, String> {
        format
            .encode()
            .map_err(|_| "Unsupported P5-C audio format.".to_string())?;
        if !(2..=5).contains(&format.profile)
            || AudioFormatV2::for_profile(format.profile, format.mode).ok() != Some(format)
            || format.channels != 2
        {
            return Err(
                "Only authenticated stereo HiFi or Hi-Res audio formats are supported.".into(),
            );
        }
        endpoint.validate()?;
        let cancellation = parent_cancel.child_token();
        check_cancelled(&cancellation).map_err(|_| "P5-C connection was cancelled.".to_string())?;
        let udp_socket = UdpSocket::bind((endpoint.local_address, 0)).map_err(|_| {
            "Unable to bind the P5-C audio socket to the selected interface.".to_string()
        })?;
        let mut tcp = connect_cancellable(endpoint.local_address, endpoint.peer, &cancellation)
            .map_err(|_| {
                "Receiver endpoint is unavailable on the selected interface.".to_string()
            })?;
        tcp.set_read_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;
        tcp.set_write_timeout(Some(SESSION_IO_POLL))
            .map_err(|_| "Unable to configure P5-C control timeout.".to_string())?;

        let binding_commitment = material.commitment();
        let (handshake, hello) = HostHandshakeV2::start(material, format)
            .map_err(|_| "P5-A v2 handshake could not start.".to_string())?;
        write_all_cancellable(&mut tcp, &hello, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 HELLO.".to_string())?;
        let mut challenge = [0u8; 104];
        read_exact_cancellable(&mut tcp, &mut challenge, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 CHALLENGE.".to_string())?;
        let (await_ready, finish) = handshake
            .accept_challenge(&challenge)
            .map_err(|_| "P5-A v2 receiver identity or proof was rejected.".to_string())?;
        write_all_cancellable(&mut tcp, &finish, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 FINISH.".to_string())?;
        let mut ready = [0u8; 40];
        read_exact_cancellable(&mut tcp, &mut ready, &cancellation)
            .map_err(|_| "P5-C peer closed during P5-A v2 READY.".to_string())?;
        let session = await_ready
            .accept_ready(&ready)
            .map_err(|_| "P5-A v2 READY proof was rejected.".to_string())?;
        if session.format != format {
            return Err("P5-A v2 returned an unexpected audio format.".into());
        }
        let session_id = session.session_id;
        let keys = session.into_keys();
        let session_keys = SessionKeys {
            audio: keys.host_to_receiver_audio,
            control_tx: keys.host_to_receiver_control,
            control_rx: keys.receiver_to_host_control,
        };
        drop(keys);
        let parameters = AudioParameters {
            sample_rate: format.sample_rate,
            channels: format.channels,
            mode: format.mode,
        };
        let queue = Arc::new(PcmQueue::for_session(
            PcmFormat {
                sample_rate: format.sample_rate,
                channels: format.channels as u16,
            },
            format.mode == 1,
        ));
        Ok(Self {
            tcp: Some(tcp),
            udp_socket: Some(udp_socket),
            sender: None,
            float_sender: None,
            pcm_payload_bytes_sent: Arc::new(AtomicU64::new(0)),
            queue,
            float_queue: Some(float_queue),
            keys: session_keys,
            parameters,
            format_v2: Some(format),
            session_id,
            binding_commitment,
            remote_udp: None,
            command_sequence: 0,
            ack_sequence: 0,
            last_command: None,
            last_ack: None,
            last_heartbeat: Instant::now(),
            state: ReceiverState::Authenticated,
            capabilities: None,
            terminal_cause: None,
            connection_intent: endpoint.connection_intent,
            background_request_id: endpoint.background_request_id,
            background_authorization_required: endpoint.background_authorization_required,
            background_authorized: false,
            cancellation,
        })
    }

    pub(super) fn authenticate_for_profile(
        endpoint: SessionEndpoint,
        binding: &BindingMaterial,
        parameters: AudioParameters,
        profile: HostAudioProfile,
        initial_float_queue: &mut Option<Arc<FloatPcmQueue>>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let HostAudioProfile::AudioV2(format) = profile else {
            if endpoint.harmony {
                return Err("鸿蒙无线播放需要 HiFi 或 Hi-Res。".into());
            }
            return Self::authenticate_on_network(
                endpoint,
                binding.duplicate_for_session(),
                parameters,
                cancellation,
            );
        };

        if AudioFormatV2::for_profile(format.profile, format.mode).ok() != Some(format)
            || !(2..=5).contains(&format.profile)
            || parameters.sample_rate != format.sample_rate
            || parameters.channels != format.channels
            || parameters.mode != format.mode
        {
            return Err(
                "Requested authenticated audio profile does not match the session parameters."
                    .into(),
            );
        }

        if !endpoint.harmony {
            let mut probe = Self::authenticate_on_network(
                endpoint,
                binding.duplicate_for_session(),
                AudioParameters {
                    sample_rate: 48_000,
                    channels: 2,
                    mode: format.mode,
                },
                cancellation.clone(),
            )?;
            probe.negotiate_capabilities_probe()?;
            let supports_v2 = probe.capabilities().is_some_and(|capabilities| {
                capabilities.compatible && capabilities.capability_bits & CAP_AUDIO_V2 != 0
            });
            let close_result = probe.close_probe_after_peer_eof();
            if let Err(error) = close_result {
                probe.close_local();
                return Err(error);
            }
            if !supports_v2 {
                return Err(if format.profile >= 4 {
                    crate::receiver::receiver_update_required(" Receiver does not advertise authenticated Float32 audio; update the Receiver or select Standard.")
                } else {
                    "Receiver does not support authenticated PCM24 audio; update the Receiver or select Standard.".into()
                });
            }
        }
        let queue = initial_float_queue
            .take()
            .unwrap_or_else(|| Arc::new(FloatPcmQueue::new(format.mode == 1)));
        Self::authenticate_v2_on_network(
            endpoint,
            binding.duplicate_for_session(),
            format,
            queue,
            cancellation,
        )
    }

    pub(crate) fn connect_loopback(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::connect_loopback_cancellable(peer, material, parameters, CancellationToken::new())
    }

    pub(crate) fn connect_on_interface(
        interface: &Ipv4Interface,
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<Self, String> {
        Self::connect_endpoint(
            SessionEndpoint::on_interface(interface, peer),
            material,
            parameters,
            CancellationToken::new(),
        )
    }

    pub(super) fn connect_loopback_cancellable(
        peer: SocketAddr,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        Self::connect_endpoint(
            SessionEndpoint::loopback(peer),
            material,
            parameters,
            cancellation,
        )
    }

    pub(super) fn connect_endpoint(
        endpoint: SessionEndpoint,
        material: BindingMaterial,
        parameters: AudioParameters,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let mut endpoint = endpoint;
        if endpoint.background_request_id.is_none() {
            endpoint.background_request_id = Some(random_request_id()?);
        }
        let mut session =
            Self::authenticate_on_network(endpoint, material, parameters, cancellation)?;
        if let Err(error) = session.negotiate_capabilities()
            .and_then(|_| if session.capabilities().is_some_and(|info| info.compatible) {
                session.authorize_background_start().and_then(|_| session.bind_audio()).and_then(|_| session.select_mode())
            } else { Err("Receiver audio or management capabilities are incompatible; update it through authorized ADB or manual installation.".into()) }) {
            session.close_local();
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn capabilities(&self) -> Option<&ReceiverCapabilities> {
        self.capabilities.as_ref()
    }

    pub(crate) fn query_background_trust(&mut self) -> Result<BackgroundTrustSnapshot, String> {
        let request_cancellation = CancellationToken::new();
        self.query_background_trust_until(
            Instant::now() + SESSION_IO_DEADLINE,
            &request_cancellation,
        )
    }

    pub(crate) fn query_background_trust_until(
        &mut self,
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if !background_trust_management_state(self.state) {
            return Err("P5-C background trust query is out of order.".into());
        }
        let state = self.state;
        let commitment = self.binding_commitment;
        let ack = self.command_until_request(
            BACKGROUND_TRUST_QUERY,
            &commitment,
            deadline,
            request_cancellation,
        )?;
        if ack.result != RESULT_OK || ack.state != state || ack.udp_port != 0 {
            return Err("Receiver rejected the background trust query.".into());
        }
        ack.background_trust
            .ok_or_else(|| "Receiver background trust response was missing.".into())
    }

    pub(crate) fn revoke_background_trust(&mut self) -> Result<BackgroundTrustSnapshot, String> {
        let request_cancellation = CancellationToken::new();
        self.revoke_background_trust_until(
            Instant::now() + SESSION_IO_DEADLINE,
            &request_cancellation,
        )
    }

    pub(crate) fn revoke_background_trust_until(
        &mut self,
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<BackgroundTrustSnapshot, String> {
        if !background_trust_management_state(self.state) {
            return Err("P5-C background trust revocation is out of order.".into());
        }
        let commitment = self.binding_commitment;
        let ack = self.command_until_request(
            BACKGROUND_TRUST_REVOKE,
            &commitment,
            deadline,
            request_cancellation,
        )?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::Stopped || ack.udp_port != 0 {
            return Err("Receiver did not confirm effective background trust revocation.".into());
        }
        let snapshot = ack
            .background_trust
            .ok_or_else(|| "Receiver background trust response was missing.".to_string())?;
        if snapshot.kind != 0 || snapshot.persistence != 0 || snapshot.remaining_ms != 0 {
            return Err("Receiver ACK did not confirm that background trust was cleared.".into());
        }
        self.close_local();
        Ok(snapshot)
    }

    pub(super) fn negotiate_capabilities(&mut self) -> Result<(), String> {
        self.negotiate_capabilities_with(REQUIRED_CAPABILITIES, true)
    }

    pub(super) fn negotiate_capabilities_probe(&mut self) -> Result<(), String> {
        self.negotiate_capabilities_with(REQUIRED_CAPABILITIES | CAP_AUDIO_V2, false)
    }

    pub(super) fn negotiate_capabilities_with(
        &mut self,
        required_capabilities: u32,
        claim_intent: bool,
    ) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated || self.capabilities.is_some() {
            return Err("P5-C capability query is out of order.".into());
        }
        let result = (|| {
            let command_sequence = self.command_sequence;
            let payload = encode_capabilities_query(
                MANAGEMENT_CONTROL_MAJOR,
                AUDIO_PROTOCOL_MAJOR,
                required_capabilities,
            );
            let frame = encode_control(
                &self.keys.control_tx,
                HOST_TO_RECEIVER_CONTROL,
                self.session_id,
                command_sequence,
                CAPABILITIES_QUERY,
                &payload,
            )?;
            let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
            write_frame(tcp, &frame, &self.cancellation)?;
            let response = read_frame(
                tcp,
                &self.keys.control_rx,
                RECEIVER_TO_HOST_CONTROL,
                self.session_id,
                &self.cancellation,
            )?;
            let info = decode_capabilities_info(&response, self.ack_sequence, command_sequence)?;
            self.command_sequence = self
                .command_sequence
                .checked_add(1)
                .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
            self.ack_sequence = self
                .ack_sequence
                .checked_add(1)
                .ok_or_else(|| "P5-C response sequence exhausted.".to_string())?;
            self.capabilities = Some(info.clone());
            if claim_intent {
                if let Some((nonce, required)) = self.connection_intent {
                    if info.capability_bits & CAP_CONNECTION_INTENT != 0 {
                        let ack = self.command(CONNECTION_INTENT, &nonce)?;
                        if ack.result != RESULT_OK || ack.state != ReceiverState::Authenticated {
                            return Err("Receiver connection intent expired or was cancelled; start receiving again on the phone.".into());
                        }
                    } else if required {
                        return Err("Receiver does not support one-click reception; use explicit connection or update through USB.".into());
                    }
                }
            }
            if self.background_authorization_required
                && info.capability_bits & CAP_BACKGROUND_START_AUTH == 0
            {
                return Err(
                    "Receiver does not support authenticated background-start authorization."
                        .into(),
                );
            }
            Ok(())
        })();
        if result.is_err() {
            self.close_local();
        }
        result
    }

    pub(super) fn close_probe_after_peer_eof(&mut self) -> Result<(), String> {
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        tcp.shutdown(std::net::Shutdown::Write)
            .map_err(|_| "Unable to close the capability probe session.".to_string())?;
        let deadline = Instant::now() + SESSION_IO_DEADLINE;
        let mut byte = [0u8; 1];
        loop {
            check_cancelled(&self.cancellation)
                .map_err(|_| "P5-C capability probe was cancelled.".to_string())?;
            if Instant::now() >= deadline {
                return Err("Receiver did not release the capability probe session.".into());
            }
            match tcp.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    return Err("Unexpected data followed the capability probe response.".into());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    return Err("Receiver closed the capability probe unexpectedly.".into());
                }
            }
        }
        self.close_local();
        Ok(())
    }

    pub(super) fn request_background_authorization(
        &mut self,
        request_id: &[u8; 16],
    ) -> Result<u8, String> {
        let ack = self.command(BACKGROUND_START_REQUEST, request_id)?;
        self.background_authorization_ack(&ack, request_id)
    }

    pub(super) fn query_background_authorization(
        &mut self,
        request_id: &[u8; 16],
    ) -> Result<u8, String> {
        let ack = self.command(BACKGROUND_START_STATUS, request_id)?;
        self.background_authorization_ack(&ack, request_id)
    }

    pub(super) fn request_background_authorization_until(
        &mut self,
        request_id: &[u8; 16],
        deadline: Instant,
    ) -> Result<u8, String> {
        let ack = self.command_until(BACKGROUND_START_REQUEST, request_id, deadline)?;
        self.background_authorization_ack(&ack, request_id)
    }

    pub(super) fn query_background_authorization_until(
        &mut self,
        request_id: &[u8; 16],
        deadline: Instant,
    ) -> Result<u8, String> {
        let ack = self.command_until(BACKGROUND_START_STATUS, request_id, deadline)?;
        self.background_authorization_ack(&ack, request_id)
    }

    pub(super) fn cancel_background_authorization(
        &mut self,
        request_id: &[u8; 16],
    ) -> Result<(), String> {
        let ack = self.command(BACKGROUND_START_CANCEL, request_id)?;
        let status = self.background_authorization_ack(&ack, request_id)?;
        if status != AUTH_CANCELLED {
            return Err("Receiver did not cancel the pending background-start request.".into());
        }
        Ok(())
    }

    pub(super) fn background_authorization_ack(
        &self,
        ack: &Ack,
        request_id: &[u8; 16],
    ) -> Result<u8, String> {
        if ack.result != RESULT_OK || ack.state != ReceiverState::Authenticated || ack.udp_port != 0
        {
            return Err("Receiver rejected background-start authorization.".into());
        }
        let (echoed_id, status) = ack
            .background_authorization
            .as_ref()
            .ok_or("Receiver background-start ACK was missing its status.")?;
        if echoed_id != request_id {
            return Err("Receiver background-start ACK request ID did not match.".into());
        }
        Ok(*status)
    }

    pub(super) fn authorize_background_start(&mut self) -> Result<(), String> {
        let supported = self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0);
        if !supported {
            return if self.background_authorization_required {
                Err(
                    "Receiver does not support authenticated background-start authorization."
                        .into(),
                )
            } else {
                Ok(())
            };
        }
        let request_id = self
            .background_request_id
            .ok_or_else(|| "P5-C background-start request ID is unavailable.".to_string())?;
        let deadline = Instant::now() + AUTHORIZATION_WAIT;
        let mut status = self.request_background_authorization_until(&request_id, deadline)?;
        let mut next_poll = Instant::now() + AUTHORIZATION_POLL;
        let mut next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        loop {
            if Instant::now() >= deadline {
                return Err(
                    "Receiver user did not approve background playback within 30 seconds.".into(),
                );
            }
            match status {
                AUTH_ONCE | AUTH_EIGHT_HOURS | AUTH_UNTIL_REVOKED => {
                    if Instant::now() >= deadline {
                        return Err(
                            "Receiver user did not approve background playback within 30 seconds."
                                .into(),
                        );
                    }
                    self.background_authorized = true;
                    return Ok(());
                }
                AUTH_DENIED => return Err("Receiver user denied background playback.".into()),
                AUTH_EXPIRED => return Err("Receiver background-start request expired.".into()),
                AUTH_CANCELLED => {
                    return Err("Receiver background-start request was cancelled.".into())
                }
                AUTH_PENDING => {}
                _ => return Err("Receiver returned an invalid background-start status.".into()),
            }
            while Instant::now() < next_poll.min(next_heartbeat).min(deadline) {
                check_cancelled(&self.cancellation)
                    .map_err(|_| "P5-C connection was cancelled.".to_string())?;
                thread::sleep(
                    SESSION_IO_POLL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            if Instant::now() >= deadline {
                return Err(
                    "Receiver user did not approve background playback within 30 seconds.".into(),
                );
            }
            if Instant::now() >= next_heartbeat {
                let heartbeat = self.command_until(HEARTBEAT, &[], deadline)?;
                if Instant::now() >= deadline
                    || heartbeat.result != RESULT_OK
                    || heartbeat.state != ReceiverState::Authenticated
                {
                    return Err(
                        "P5-C authenticated heartbeat failed during background-start approval."
                            .into(),
                    );
                }
                self.last_heartbeat = Instant::now();
                self.state = heartbeat.state;
                next_heartbeat += HEARTBEAT_INTERVAL;
            }
            if Instant::now() >= next_poll {
                status = self.query_background_authorization_until(&request_id, deadline)?;
                if Instant::now() >= deadline {
                    return Err(
                        "Receiver user did not approve background playback within 30 seconds."
                            .into(),
                    );
                }
                next_poll += AUTHORIZATION_POLL;
            }
        }
    }

    pub(crate) fn state(&self) -> ReceiverState {
        self.state
    }

    pub(crate) fn session_id(&self) -> u64 {
        self.session_id
    }

    pub(crate) fn pcm_queue(&self) -> Arc<PcmQueue> {
        self.queue.clone()
    }

    pub(crate) fn audio_queue(&self) -> SessionAudioQueue {
        self.float_queue
            .as_ref()
            .zip(self.format_v2)
            .map(|(queue, format)| SessionAudioQueue::Float32(queue.clone(), format))
            .unwrap_or_else(|| SessionAudioQueue::Pcm16(self.queue.clone()))
    }

    pub(crate) fn is_v2_audio(&self) -> bool {
        self.format_v2.is_some()
    }

    pub(crate) fn pcm_payload_bytes_sent(&self) -> u64 {
        self.pcm_payload_bytes_sent.load(Ordering::Relaxed)
    }

    pub(super) fn check_sender(&mut self) -> Result<(), String> {
        if let Some(sender) = self.sender.as_mut() {
            if let Some(error) = sender.take_finished_error() {
                return Err(error);
            }
        }
        if let Some(sender) = self.float_sender.as_mut() {
            if let Some(error) = sender.take_finished_error() {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn bind_audio(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Authenticated {
            return Err("P5-C BIND is out of order.".into());
        }
        let required_audio = self.format_v2.map_or(0, |format| {
            CAP_AUDIO_V2
                | if format.profile >= 4 {
                    CAP_AUDIO_FLOAT32
                } else {
                    0
                }
        });
        if self.capabilities.as_ref().is_some_and(|info| {
            !info.compatible || info.capability_bits & required_audio != required_audio
        }) {
            return Err(
                if self.format_v2.is_some_and(|format| format.profile >= 4) {
                    crate::receiver::receiver_update_required(" Receiver does not advertise the requested authenticated Float32 audio format; update the Receiver or select Standard.")
                } else if self.format_v2.is_some() {
                    crate::receiver::receiver_update_required(" Receiver does not support the requested authenticated audio format; update the Receiver or select Standard.")
                } else {
                    "Receiver audio capabilities are incompatible; update it or select Standard."
                        .into()
                },
            );
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_CONNECTION_INTENT != 0)
            && self.connection_intent.is_none()
            && !self.background_authorized
        {
            return Err("receiver_requires_connection_offer: Receiver requires a phone-initiated connection offer before BIND.".into());
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0)
            && !self.background_authorized
        {
            return Err("P5-C BIND requires an approved background-start request.".into());
        }
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "Unable to read P5-C UDP source port.".to_string())?
            .port();
        let ack = self.command(BIND, &port.to_be_bytes())?;
        if ack.result != RESULT_OK
            || ack.state != ReceiverState::TransportConnected
            || ack.udp_port == 0
        {
            return Err(format!(
                "P5-C receiver did not confirm the authenticated UDP peer (result={}, state={:?}, udp_port={}).",
                ack.result, ack.state, ack.udp_port
            ));
        }
        let tcp_peer = self
            .tcp
            .as_ref()
            .ok_or("P5-C control channel is closed.")?
            .peer_addr()
            .map_err(|_| "Unable to read P5-C peer address.".to_string())?;
        self.remote_udp = Some(SocketAddr::new(tcp_peer.ip(), ack.udp_port));
        self.state = ack.state;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn bind_audio_without_connection_intent_for_test(&mut self) -> Result<bool, String> {
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "Unable to read P5-C UDP source port.".to_string())?
            .port();
        self.command(BIND, &port.to_be_bytes())
            .map(|ack| ack.result == RESULT_REJECTED)
    }

    pub(crate) fn select_mode(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected {
            return Err("P5-C mode selection is out of order.".into());
        }
        let ack = self.command(SELECT_MODE, &[self.parameters.mode])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::TransportConnected {
            return Err("P5-C receiver rejected the authenticated mode.".into());
        }
        Ok(())
    }

    pub(crate) fn start(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected || self.sender.is_some() {
            return Err("P5-C START is out of order.".into());
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_START_AUTH != 0)
            && !self.background_authorized
        {
            return Err("P5-C START requires an approved background-start request.".into());
        }
        let ack = self.command(START, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            self.close_local();
            return Err(START_NOT_CONFIRMED.into());
        }
        let result = (|| {
            let socket = self
                .udp_socket
                .as_ref()
                .ok_or("P5-C UDP socket is closed.")?
                .try_clone()
                .map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material =
                UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            let send_loss_ledger = self.capabilities.as_ref().is_some_and(|info| {
                info.capability_bits & (CAP_SOURCE_FRAME_LOSS | CAP_SOURCE_SEGMENT_START)
                    == (CAP_SOURCE_FRAME_LOSS | CAP_SOURCE_SEGMENT_START)
            });
            if let (Some(queue), Some(format)) = (&self.float_queue, self.format_v2) {
                self.float_sender = Some(FloatUdpSender::spawn_authenticated(
                    remote,
                    queue.clone(),
                    socket,
                    material,
                    format,
                    self.pcm_payload_bytes_sent.clone(),
                    send_loss_ledger,
                )?);
            } else {
                self.sender = Some(UdpSender::spawn_authenticated(
                    remote,
                    self.queue.clone(),
                    socket,
                    material,
                    self.parameters,
                    self.pcm_payload_bytes_sent.clone(),
                    send_loss_ledger,
                )?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.close_local();
            return Err(error);
        }
        self.state = ack.state;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn start_without_format_for_test(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::TransportConnected || self.sender.is_some() {
            return Err("P5-C START is out of order.".into());
        }
        let ack = self.command(START, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            self.close_local();
            return Err(START_NOT_CONFIRMED.into());
        }
        let result = (|| {
            let socket = self
                .udp_socket
                .as_ref()
                .ok_or("P5-C UDP socket is closed.")?
                .try_clone()
                .map_err(|_| "Unable to transfer the authenticated UDP socket.".to_string())?;
            let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
            let material =
                UdpSessionMaterial::from_authenticated(self.session_id, &self.keys.audio);
            self.sender = Some(UdpSender::spawn_authenticated_without_format_for_test(
                remote,
                self.queue.clone(),
                socket,
                material,
                self.parameters,
                self.pcm_payload_bytes_sent.clone(),
            )?);
            Ok(())
        })();
        if let Err(error) = result {
            self.close_local();
            return Err(error);
        }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn pause(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::ReceiverRunning && self.state != ReceiverState::Streaming {
            return Err("P5-C PAUSE is out of order.".into());
        }
        let ack = self.command(PAUSE, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::Paused {
            return Err("P5-C PAUSE was rejected.".into());
        }
        self.queue.end_source_segment();
        self.queue.clear_queued();
        if let Some(queue) = &self.float_queue {
            queue.end_source_segment();
            queue.clear_queued();
        }
        self.state = ack.state;
        if let Some(sender) = &self.float_sender {
            sender.expect_queue_stop();
        }
        Ok(())
    }

    pub(crate) fn resume(&mut self) -> Result<(), String> {
        if self.state != ReceiverState::Paused {
            return Err("P5-C RESUME is out of order.".into());
        }
        let ack = self.command(RESUME, &[])?;
        if ack.result != RESULT_OK || ack.state != ReceiverState::ReceiverRunning {
            return Err("P5-C RESUME was rejected.".into());
        }
        if let Some(sender) = &self.float_sender {
            sender.resume_after_ack()?;
        }
        self.queue.clear_queued();
        self.queue.begin_source_segment();
        if let Some(queue) = &self.float_queue {
            queue.clear_queued();
            queue.begin_source_segment();
        }
        self.state = ack.state;
        Ok(())
    }

    pub(crate) fn heartbeat(&mut self) -> Result<ReceiverState, String> {
        let ack = self.command(HEARTBEAT, &[])?;
        if ack.result == RESULT_USER_STOPPED {
            if ack.state != ReceiverState::Stopped
                || !self
                    .capabilities
                    .as_ref()
                    .is_some_and(|caps| caps.capability_bits & CAP_TERMINAL_CAUSE != 0)
            {
                return Err("Receiver returned an invalid authenticated terminal cause.".into());
            }
            self.terminal_cause = Some(TerminalCause::ReceiverUserStop);
        } else if ack.result != RESULT_OK {
            return Err("P5-C heartbeat was rejected.".into());
        }
        self.last_heartbeat = Instant::now();
        self.state = ack.state;
        Ok(self.state)
    }

    pub(crate) fn terminal_cause(&self) -> Option<TerminalCause> {
        self.terminal_cause
    }

    pub(super) fn playback_progress(&mut self) -> Result<Option<PlaybackDiagnostics>, String> {
        if !self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_PLAYBACK_PROGRESS != 0)
        {
            return Ok(None);
        }
        let schema = self
            .capabilities
            .as_ref()
            .map_or(0, |info| requested_progress_schema(info.capability_bits));
        let request = [schema];
        let ack = self.command(
            PLAYBACK_PROGRESS_QUERY,
            if schema != 0 { &request } else { &[] },
        )?;
        if ack.result != RESULT_OK {
            return Err("P5-C playback progress query was rejected.".into());
        }
        let playback = ack
            .playback_progress
            .ok_or_else(|| "Receiver playback progress response was missing.".to_string())?;
        if let Some(revision) = playback.applied_loss_ledger_revision {
            match self.audio_queue() {
                SessionAudioQueue::Pcm16(queue) => queue.acknowledge_source_revision(revision),
                SessionAudioQueue::Float32(queue, _) => queue.acknowledge_source_revision(revision),
            }
        }
        Ok(Some(playback))
    }

    pub(crate) fn check_liveness_at(&mut self, now: Instant) -> Result<(), String> {
        if now.saturating_duration_since(self.last_heartbeat) > HEARTBEAT_TIMEOUT {
            self.close_local();
            return Err("P5-C authenticated heartbeat expired.".into());
        }
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        let result = if self.state == ReceiverState::Stopped {
            Ok(())
        } else {
            match self.command(STOP, &[]) {
                Ok(ack) if ack.result == RESULT_OK && ack.state == ReceiverState::Stopped => {
                    self.state = ReceiverState::Stopped;
                    Ok(())
                }
                Ok(_) => Err("P5-C STOP was rejected.".into()),
                Err(error) => Err(error),
            }
        };
        self.close_local();
        result
    }

    pub(super) fn command(&mut self, kind: u8, payload: &[u8]) -> Result<Ack, String> {
        let result = self.command_inner(kind, payload);
        if result.is_err() {
            self.close_local();
        }
        result
    }

    pub(super) fn command_until(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Ack, String> {
        let result = self.command_inner_until(kind, payload, deadline);
        if result.is_err() {
            self.close_local();
        }
        result
    }

    pub(super) fn command_until_request(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
        request_cancellation: &CancellationToken,
    ) -> Result<Ack, String> {
        let mut bytes_written = false;
        let result = self.command_inner_until_request(
            kind,
            payload,
            deadline,
            request_cancellation,
            &mut bytes_written,
        );
        if result.is_err() && bytes_written {
            self.close_local();
        }
        result
    }

    pub(super) fn command_inner(&mut self, kind: u8, payload: &[u8]) -> Result<Ack, String> {
        if matches!(kind, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE)
            && !self
                .capabilities
                .as_ref()
                .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
        {
            return Err("Receiver does not support background trust management.".into());
        }
        if matches!(kind, BACKGROUND_TRUST_QUERY | BACKGROUND_TRUST_REVOKE)
            && (payload.len() != 32 || payload != self.binding_commitment.as_slice())
        {
            return Err(
                "P5-C background trust request must target the authenticated binding.".into(),
            );
        }
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame(tcp, &frame, &self.cancellation)?;
        let ack_frame = read_frame(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
        )?;
        let ack = decode_ack_with_format_schema(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
            progress_schema_for_request(kind, payload),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    pub(super) fn command_inner_until(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Ack, String> {
        if Instant::now() >= deadline {
            return Err("P5-C approval deadline expired.".into());
        }
        let io_deadline = deadline.min(Instant::now() + SESSION_IO_DEADLINE);
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame_until(tcp, &frame, &self.cancellation, io_deadline)?;
        let ack_frame = read_frame_until(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
            io_deadline,
        )?;
        if Instant::now() >= deadline {
            return Err("P5-C approval deadline expired.".into());
        }
        let ack = decode_ack_with_format_schema(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
            progress_schema_for_request(kind, payload),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    pub(super) fn command_inner_until_request(
        &mut self,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
        request_cancellation: &CancellationToken,
        bytes_written: &mut bool,
    ) -> Result<Ack, String> {
        if Instant::now() >= deadline || request_cancellation.is_cancelled() {
            return Err(
                "P5-C background trust request expired or was cancelled before sending.".into(),
            );
        }
        if !self
            .capabilities
            .as_ref()
            .is_some_and(|info| info.capability_bits & CAP_BACKGROUND_TRUST_MANAGEMENT != 0)
        {
            return Err("Receiver does not support background trust management.".into());
        }
        if payload.len() != 32 || payload != self.binding_commitment.as_slice() {
            return Err(
                "P5-C background trust request must target the authenticated binding.".into(),
            );
        }
        let sequence = self.command_sequence;
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            sequence,
            kind,
            payload,
        )?;
        let tcp = self.tcp.as_mut().ok_or("P5-C control channel is closed.")?;
        write_frame_until_request(
            tcp,
            &frame,
            &self.cancellation,
            request_cancellation,
            deadline,
            bytes_written,
        )?;
        let ack_frame = read_frame_until_request(
            tcp,
            &self.keys.control_rx,
            RECEIVER_TO_HOST_CONTROL,
            self.session_id,
            &self.cancellation,
            request_cancellation,
            deadline,
        )?;
        let ack = decode_ack_with_format_schema(
            &ack_frame,
            self.ack_sequence,
            kind,
            sequence,
            self.format_v2.is_some(),
            progress_schema_for_request(kind, payload),
        )?;
        self.record_terminal_cause(&ack)?;
        if ack
            .background_trust
            .as_ref()
            .is_some_and(|trust| trust.commitment != self.binding_commitment)
        {
            return Err(
                "Receiver background trust ACK binding did not match the authenticated session."
                    .into(),
            );
        }
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C command sequence exhausted.".to_string())?;
        self.ack_sequence = self
            .ack_sequence
            .checked_add(1)
            .ok_or_else(|| "P5-C ACK sequence exhausted.".to_string())?;
        self.last_command = Some(frame);
        self.last_ack = Some(ack_frame);
        Ok(ack)
    }

    pub(super) fn record_terminal_cause(&mut self, ack: &Ack) -> Result<(), String> {
        if ack.result != RESULT_USER_STOPPED {
            return Ok(());
        }
        if ack.state != ReceiverState::Stopped
            || !self
                .capabilities
                .as_ref()
                .is_some_and(|caps| caps.capability_bits & CAP_TERMINAL_CAUSE != 0)
        {
            return Err("P5-C Receiver terminal cause was invalid.".into());
        }
        self.terminal_cause = Some(TerminalCause::ReceiverUserStop);
        self.state = ReceiverState::Stopped;
        Ok(())
    }

    pub(super) fn close_local(&mut self) {
        self.cancellation.cancel();
        self.queue.stop_and_clear();
        if let Some(queue) = &self.float_queue {
            queue.stop_and_clear();
        }
        if let Some(sender) = self.sender.take() {
            let _ = sender.stop();
        }
        if let Some(sender) = self.float_sender.take() {
            let _ = sender.stop();
        }
        drop(self.udp_socket.take());
        if let Some(tcp) = self.tcp.take() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
        self.keys.audio.fill(0);
        self.keys.control_tx.fill(0);
        self.keys.control_rx.fill(0);
        self.state = ReceiverState::Stopped;
    }

    #[cfg(test)]
    pub(super) fn resend_last_for_test(&mut self) -> Result<(), String> {
        let frame = self
            .last_command
            .as_ref()
            .ok_or("No P5-C command to repeat.")?
            .clone();
        let expected_ack = self
            .last_ack
            .as_ref()
            .ok_or("No P5-C ACK to compare.")?
            .clone();
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        let actual_ack = read_raw_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &self.cancellation,
        )?;
        if actual_ack != expected_ack {
            return Err("P5-C duplicate command changed its ACK.".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn send_bad_mac_for_test(&mut self) -> Result<(), String> {
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "UDP socket unavailable".to_string())?
            .port();
        let mut frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            self.command_sequence,
            BIND,
            &port.to_be_bytes(),
        )?;
        *frame.last_mut().unwrap() ^= 0x80;
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    pub(super) fn send_bad_capabilities_query_for_test(
        &mut self,
        bad_mac: bool,
    ) -> Result<(), String> {
        let mut payload = encode_capabilities_query(
            MANAGEMENT_CONTROL_MAJOR,
            AUDIO_PROTOCOL_MAJOR,
            REQUIRED_CAPABILITIES,
        );
        payload[3] = 1;
        let mut frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id,
            self.command_sequence,
            CAPABILITIES_QUERY,
            &payload,
        )?;
        if bad_mac {
            *frame.last_mut().unwrap() ^= 1;
        }
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    pub(super) fn send_old_session_for_test(&mut self) -> Result<(), String> {
        let port = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .local_addr()
            .map_err(|_| "UDP socket unavailable".to_string())?
            .port();
        let frame = encode_control(
            &self.keys.control_tx,
            HOST_TO_RECEIVER_CONTROL,
            self.session_id.wrapping_sub(1),
            self.command_sequence,
            BIND,
            &port.to_be_bytes(),
        )?;
        write_frame(
            self.tcp.as_mut().ok_or("P5-C control channel is closed.")?,
            &frame,
            &self.cancellation,
        )?;
        expect_peer_close(self.tcp.as_mut().ok_or("P5-C control channel is closed.")?)
    }

    #[cfg(test)]
    pub(super) fn send_old_audio_for_test(&self) -> Result<(), String> {
        let format = self.parameters;
        let mut encoder = PacketEncoder::with_sequence(
            &self.keys.audio,
            self.session_id.wrapping_sub(1),
            HOST_TO_RECEIVER,
            0,
        );
        let frame = encoder
            .encode_next(
                PacketType::Format,
                0,
                &PcmFormat {
                    sample_rate: format.sample_rate,
                    channels: format.channels as u16,
                }
                .encode()
                .map_err(|_| "invalid session PCM format")?,
            )
            .map_err(|_| "unable to encode synthetic old-session packet")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .send_to(&frame, remote)
            .map_err(|_| "unable to send old-session packet")?;
        if sent != frame.len() {
            return Err("old-session datagram send was incomplete".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn send_format_packet_for_test(&self, sample_rate: u32) -> Result<(), String> {
        let payload = PcmFormat {
            sample_rate,
            channels: self.parameters.channels as u16,
        }
        .encode()
        .map_err(|_| "invalid synthetic test format")?;
        let mut encoder =
            PacketEncoder::with_sequence(&self.keys.audio, self.session_id, HOST_TO_RECEIVER, 0);
        let packet = encoder
            .encode_next(PacketType::Format, 0, &payload)
            .map_err(|_| "unable to encode synthetic FORMAT")?;
        let remote = self.remote_udp.ok_or("P5-C UDP peer was not confirmed.")?;
        let sent = self
            .udp_socket
            .as_ref()
            .ok_or("P5-C UDP socket is closed.")?
            .send_to(&packet, remote)
            .map_err(|_| "unable to send synthetic FORMAT")?;
        if sent != packet.len() {
            return Err("synthetic FORMAT datagram send was incomplete".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn remove_udp_socket_for_test(&mut self) {
        drop(self.udp_socket.take());
    }

    #[cfg(test)]
    pub(super) fn remove_control_for_test(&mut self) {
        if let Some(tcp) = self.tcp.take() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
    }
}
