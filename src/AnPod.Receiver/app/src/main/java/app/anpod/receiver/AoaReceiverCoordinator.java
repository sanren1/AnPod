package app.anpod.receiver;

import java.io.IOException;
import android.os.ParcelFileDescriptor;
import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/** One authenticated AOA session. The coordinator owns transport, keys, and its audio context. */
final class AoaReceiverCoordinator implements AutoCloseable {
    interface PlaybackEndpoint {
        boolean isCurrent(String ownerId, long generation);
        boolean supportsAudio(UdpPacketDecoder.AudioFormatV2 format);
        void start(AoaSessionContext context) throws IOException;
        void pause() throws IOException;
        void resume() throws IOException;
        void stop();
        boolean streaming() throws IOException;
        byte[] playbackProgressPayloadV2();
    }

    private static final Object OWNER_LOCK = new Object();
    private static final long FINISH_TIMEOUT_NANOS = TimeUnit.SECONDS.toNanos(3);
    private static final long HEARTBEAT_TIMEOUT_NANOS = TimeUnit.SECONDS.toNanos(5);
    private static final long WRITE_TIMEOUT_MILLIS = 3_000;
    private static final int RESULT_USER_STOPPED = 3;
    private static AoaReceiverCoordinator owner;

    private final AoaAccessoryIo io;
    private final PairingProtocol.BindingMaterial binding;
    private final PlaybackEndpoint playback;
    private final String ownerId;
    private final long generation;
    private final long helloTimeoutMillis;
    private final ArrayBlockingQueue<WriteRequest> writes = new ArrayBlockingQueue<>(8);
    private final Object lifecycle = new Object();
    private final CountDownLatch terminated = new CountDownLatch(1);
    private final Thread reader;
    private final Thread writer;
    private final Thread watchdog;
    private volatile boolean cancelled;
    private volatile boolean stopAfterAck;
    private volatile AoaSessionContext audio;
    private volatile PairingProtocol.SessionKeys keys;
    private volatile UdpPacketDecoder.AudioFormatV2 format;
    private volatile long sessionId;
    private volatile long lastControlAtNanos;
    private volatile long startupDeadlineNanos;
    private volatile boolean authenticated;
    private volatile boolean playbackStarted;
    private volatile boolean playbackEverStarted;
    private volatile boolean hostStopped;
    private volatile IOException failure;
    private volatile IOException cleanupFailure;
    private volatile int state = P5cReceiverCoordinator.STATE_AUTHENTICATED;
    private volatile int supportedCapabilities;
    private volatile boolean userStopRequested;
    private volatile boolean restartRequested;

    static AoaReceiverCoordinator start(AoaAccessoryIo io, PairingProtocol.BindingMaterial savedBinding,
            PlaybackEndpoint playback, String ownerId, long generation) throws IOException {
        return start(io, savedBinding, playback, ownerId, generation, TimeUnit.SECONDS.toMillis(30));
    }

    static AoaReceiverCoordinator start(AoaAccessoryIo io, PairingProtocol.BindingMaterial savedBinding,
            PlaybackEndpoint playback, String ownerId, long generation, long helloTimeoutMillis) throws IOException {
        if (io == null || savedBinding == null || playback == null || ownerId == null)
            throw new NullPointerException();
        if (helloTimeoutMillis <= 0) throw new IllegalArgumentException("hello timeout must be positive");
        synchronized (OWNER_LOCK) {
            ensureNoOwner();
            return startLocked(io, savedBinding, playback, ownerId, generation, helloTimeoutMillis);
        }
    }

    /** Production entry keeps permission, attachment, and the actual open under the sole-owner lock. */
    static AoaReceiverCoordinator openAccessory(UsbManager manager, UsbAccessory accessory,
            PairingProtocol.BindingMaterial savedBinding, PlaybackEndpoint playback,
            String ownerId, long generation) throws IOException {
        if (manager == null || accessory == null || savedBinding == null || playback == null || ownerId == null)
            throw new NullPointerException();
        synchronized (OWNER_LOCK) {
            if (owner != null) {
                savedBinding.close();
                throw new IOException("AoaSessionCleanupPending");
            }
            boolean authorized;
            try {
                authorized = isProductAccessory(accessory) && manager.hasPermission(accessory)
                        && isCurrentlyAttached(manager, accessory);
            } catch (RuntimeException error) {
                savedBinding.close();
                throw new IOException("UsbAccessoryPermissionCheckFailed", error);
            }
            if (!authorized) {
                savedBinding.close();
                throw new IOException("UsbAccessoryPermissionOrAttachmentLost");
            }
            if (!playback.isCurrent(ownerId, generation)) {
                savedBinding.close();
                throw new IOException("AoaOwnerGenerationExpired");
            }

            ParcelFileDescriptor descriptor;
            try { descriptor = manager.openAccessory(accessory); }
            catch (RuntimeException error) {
                savedBinding.close();
                throw new IOException("UsbAccessoryOpenFailed", error);
            }
            if (descriptor == null) {
                savedBinding.close();
                throw new IOException("UsbAccessoryOpenFailed");
            }
            AoaAccessoryIo io = null;
            try {
                if (!manager.hasPermission(accessory) || !isCurrentlyAttached(manager, accessory)
                        || !playback.isCurrent(ownerId, generation))
                    throw new IOException("UsbAccessoryChangedDuringOpen");
                io = AoaAccessoryIo.open(descriptor);
                return startLocked(io, savedBinding, playback, ownerId, generation,
                        TimeUnit.SECONDS.toMillis(30));
            } catch (IOException | RuntimeException error) {
                if (io != null) io.cancel();
                else try { descriptor.close(); } catch (IOException closeError) { error.addSuppressed(closeError); }
                savedBinding.close();
                if (error instanceof IOException) throw (IOException) error;
                throw new IOException("UsbAccessoryOpenFailed", error);
            }
        }
    }

    static boolean isProductAccessory(UsbAccessory accessory) {
        return accessory != null && "AnPod".equals(accessory.getManufacturer())
                && "AnPod USB Direct".equals(accessory.getModel());
    }

    private static boolean isCurrentlyAttached(UsbManager manager, UsbAccessory expected) {
        UsbAccessory[] attached = manager.getAccessoryList();
        if (attached == null) return false;
        for (UsbAccessory candidate : attached)
            if (manager.hasPermission(candidate) && expected.equals(candidate) && isProductAccessory(candidate)) return true;
        return false;
    }

    private static void ensureNoOwner() throws IOException {
        if (owner != null) throw new IOException("AoaSessionCleanupPending");
    }

    static boolean isAvailable() {
        synchronized (OWNER_LOCK) { return owner == null; }
    }

    private static AoaReceiverCoordinator startLocked(AoaAccessoryIo io,
            PairingProtocol.BindingMaterial savedBinding, PlaybackEndpoint playback,
            String ownerId, long generation, long helloTimeoutMillis) {
        AoaReceiverCoordinator next = new AoaReceiverCoordinator(io, savedBinding, playback,
                ownerId, generation, helloTimeoutMillis);
        owner = next;
        next.startWorkers();
        return next;
    }

    private AoaReceiverCoordinator(AoaAccessoryIo io, PairingProtocol.BindingMaterial binding,
            PlaybackEndpoint playback, String ownerId, long generation, long helloTimeoutMillis) {
        this.io = io;
        this.binding = binding;
        this.playback = playback;
        this.ownerId = ownerId;
        this.generation = generation;
        this.helloTimeoutMillis = helloTimeoutMillis;
        reader = new Thread(this::readLoop, "AnPod AOA control reader");
        writer = new Thread(this::writeLoop, "AnPod AOA stream writer");
        watchdog = new Thread(this::watchdogLoop, "AnPod AOA session watchdog");
        watchdog.setDaemon(true);
    }

    private void startWorkers() {
        startupDeadlineNanos = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(helloTimeoutMillis);
        writer.start();
        reader.start();
        watchdog.start();
    }

    @Override public void close() { cancel("AoaLocalStop"); }

    boolean awaitTermination(long timeout, TimeUnit unit) throws InterruptedException {
        return terminated.await(timeout, unit);
    }

    String failureMessage() {
        IOException current = failure;
        if (current == null) current = cleanupFailure;
        if (current == null) return "";
        StringBuilder message = new StringBuilder(current.getMessage() == null
                ? current.getClass().getSimpleName() : current.getMessage());
        if (cleanupFailure != null && cleanupFailure != current)
            message.append("; cleanup: ").append(cleanupFailure.getMessage());
        for (Throwable suppressed : current.getSuppressed())
            message.append("; cleanup: ").append(suppressed.getMessage());
        return message.toString();
    }

    /** Safe on the main thread: invalidates and signals only; joining and key destruction happen elsewhere. */
    void cancel(String reason) {
        synchronized (lifecycle) {
            if (cancelled) return;
            cancelled = true;
        }
        AoaSessionContext context = audio;
        if (context != null) context.stopQueue();
        io.cancel();
        reader.interrupt();
        writer.interrupt();
        watchdog.interrupt();
        Thread cleanup = new Thread(this::cleanup, "AnPod AOA owner cleanup");
        cleanup.setDaemon(true);
        cleanup.start();
    }

    boolean requestUserStop() {
        if (cancelled || !authenticated) return false;
        userStopRequested = true;
        return true;
    }

    boolean restartRequested() { return restartRequested; }

    boolean shouldRecoverUnexpectedFailure() {
        return shouldRecoverUnexpectedFailure(playbackEverStarted, userStopRequested,
                hostStopped, restartRequested, failure != null);
    }

    static boolean shouldRecoverUnexpectedFailure(boolean playbackEverStarted,
            boolean userStopRequested, boolean hostStopped, boolean restartRequested,
            boolean failureObserved) {
        return playbackEverStarted && !userStopRequested && !hostStopped
                && !restartRequested && failureObserved;
    }

    private void readLoop() {
        try {
            if (!playback.isCurrent(ownerId, generation)) throw new IOException("AOA owner generation expired");
            PairingProtocol.SessionKeys sessionKeys = authenticate();
            keys = sessionKeys;
            authenticated = true;
            lastControlAtNanos = System.nanoTime();
            controlLoop(sessionKeys);
        } catch (IOException | PairingProtocol.Failure | RuntimeException error) {
            fail(error instanceof IOException ? (IOException) error : new IOException("AOA session failed", error));
        } finally {
            cancel("AoaSessionEnded");
        }
    }

    private PairingProtocol.SessionKeys authenticate() throws IOException, PairingProtocol.Failure {
        AnpuRecordCodec.Record hello = readExpected(AnpuRecordCodec.HANDSHAKE);
        startupDeadlineNanos = System.nanoTime() + FINISH_TIMEOUT_NANOS;
        PairingProtocol.BindingMaterial derived = binding.deriveAoa();
        PairingProtocol.V2.ReceiverAwaitFinish handshake;
        try { handshake = PairingProtocol.V2.Receiver.acceptHello(derived, hello.body); }
        catch (PairingProtocol.Failure | RuntimeException error) { derived.close(); throw error; }
        try (PairingProtocol.V2.ReceiverAwaitFinish awaiting = handshake) {
            writeRecord(AnpuRecordCodec.HANDSHAKE, awaiting.challenge);
            AnpuRecordCodec.Record finish = readExpected(AnpuRecordCodec.HANDSHAKE);
            PairingProtocol.V2.FinishResult completed = awaiting.acceptFinish(finish.body);
            try (PairingProtocol.V2.AuthenticatedSessionV2 session = completed.session) {
                format = session.format;
                sessionId = session.sessionId;
                writeRecord(AnpuRecordCodec.HANDSHAKE, completed.ready);
                return session.takeKeys();
            }
        }
    }

    private void controlLoop(PairingProtocol.SessionKeys sessionKeys) throws IOException {
        long expectedCommand = 0;
        long ackSequence = 0;
        boolean modeSelected = false;
        boolean capabilitiesAccepted = false;
        boolean bound = false;
        while (!cancelled) {
            if (!playback.isCurrent(ownerId, generation)) throw new IOException("AOA owner generation expired");
            AnpuRecordCodec.Record record = io.readRecord();
            if (!playback.isCurrent(ownerId, generation)) throw new IOException("AOA owner generation expired");
            if (record.type == AnpuRecordCodec.AUDIO) {
                if (!playbackStarted || audio == null || state == P5cReceiverCoordinator.STATE_STOPPED)
                    throw new IOException("AOA audio arrived before START");
                AnpdAudioIngress.Result result = audio.ingress.accept(record.body, record.body.length);
                if (result != AnpdAudioIngress.Result.FORMAT && result != AnpdAudioIngress.Result.AUDIO)
                    throw new IOException("AOA authenticated audio rejected: " + result);
                continue;
            }
            if (record.type != AnpuRecordCodec.CONTROL) throw new IOException("unexpected AOA record type");
            PairingProtocol.SessionKeys keysSnapshot = sessionKeys;
            P5cControlProtocol.Frame command = P5cControlProtocol.decodeRaw(record.body,
                    keysSnapshot.hostToReceiverControl, P5cControlProtocol.HOST_TO_RECEIVER, sessionId);
            if (command.sequence != expectedCommand || expectedCommand == Long.MAX_VALUE)
                throw new IOException("AOA P5-C replay or sequence gap");
            expectedCommand++;
            lastControlAtNanos = System.nanoTime();
            if (!capabilitiesAccepted) {
                if (command.type != P5cControlProtocol.CAPABILITIES_QUERY || command.payload.length != 8
                        || command.payload[3] != 0)
                    throw new IOException("AOA capability query must be first");
                int requestedControl = P5cControlProtocol.getUnsignedShort(command.payload, 0);
                int requestedAudio = command.payload[2] & 0xff;
                int required = readInt(command.payload, 4);
                int supported = P5cControlProtocol.CAP_AUTHENTICATED_STATUS
                        | (playback.supportsAudio(format) ? P5cControlProtocol.CAP_AUDIO_V2 : 0)
                        | P5cControlProtocol.CAP_PLAYBACK_PROGRESS
                        | P5cControlProtocol.CAP_TERMINAL_CAUSE
                        | P5cControlProtocol.CAP_AOA_SESSION_RESTART;
                boolean compatible = requestedControl == P5cControlProtocol.MANAGEMENT_CONTROL_MAJOR
                        && requestedAudio == P5cControlProtocol.AUDIO_PROTOCOL_MAJOR
                        && (required & ~supported) == 0 && playback.supportsAudio(format);
                byte[] info = P5cControlProtocol.capabilitiesInfoPayload(command.sequence,
                        P5cControlProtocol.MANAGEMENT_CONTROL_MAJOR, P5cControlProtocol.AUDIO_PROTOCOL_MAJOR,
                        compatible ? P5cReceiverCoordinator.RESULT_OK : P5cReceiverCoordinator.RESULT_REJECTED,
                        supported, BuildConfig.VERSION_CODE);
                writeRecord(AnpuRecordCodec.CONTROL, P5cControlProtocol.encode(keysSnapshot.receiverToHostControl,
                        P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++,
                        P5cControlProtocol.CAPABILITIES_INFO, info));
                if (!compatible) throw new IOException("AOA capabilities rejected");
                supportedCapabilities = supported;
                capabilitiesAccepted = true;
                state = P5cReceiverCoordinator.STATE_AUTHENTICATED;
                continue;
            }

            byte[] extra = new byte[0];
            int port = 0;
            boolean terminate = false;
            int result = P5cReceiverCoordinator.RESULT_OK;
            if (userStopRequested) {
                result = RESULT_USER_STOPPED;
                state = P5cReceiverCoordinator.STATE_STOPPED;
                terminate = true;
            } else switch (command.type) {
                case P5cControlProtocol.BIND:
                    if (bound || command.payload.length != 2 || command.payload[0] != 0 || command.payload[1] != 0)
                        throw new IOException("AOA BIND must contain zero port");
                    bound = true;
                    state = P5cReceiverCoordinator.STATE_TRANSPORT_CONNECTED;
                    break;
                case P5cControlProtocol.SELECT_MODE:
                    if (state != P5cReceiverCoordinator.STATE_TRANSPORT_CONNECTED || !bound || format == null || command.payload.length != 1
                            || (command.payload[0] & 0xff) != format.mode)
                        throw new IOException("AOA selected mode differs from descriptor");
                    modeSelected = true;
                    break;
                case P5cControlProtocol.START:
                    if (!bound || !modeSelected || format == null || command.payload.length != 0 || playbackStarted)
                        throw new IOException("invalid AOA START state");
                    AoaSessionContext context = new AoaSessionContext(sessionId, keysSnapshot.hostToReceiverAudio, format);
                    audio = context;
                    try { playback.start(context); }
                    catch (IOException | RuntimeException error) {
                        context.stopQueue();
                        throw error instanceof IOException ? (IOException) error : new IOException("AOA playback start failed", error);
                    }
                    synchronized (lifecycle) { playbackStarted = true; }
                    playbackEverStarted = true;
                    if (cancelled) throw new IOException("AOA session cancelled during playback start");
                    state = P5cReceiverCoordinator.STATE_RECEIVER_RUNNING;
                    break;
                case P5cControlProtocol.PAUSE:
                    if (command.payload.length != 0 || !playbackStarted
                            || state != P5cReceiverCoordinator.STATE_RECEIVER_RUNNING
                            && state != P5cReceiverCoordinator.STATE_STREAMING
                            && state != P5cReceiverCoordinator.STATE_PAUSED)
                        throw new IOException("invalid AOA PAUSE state");
                    if (state != P5cReceiverCoordinator.STATE_PAUSED) playback.pause();
                    state = P5cReceiverCoordinator.STATE_PAUSED;
                    break;
                case P5cControlProtocol.RESUME:
                    if (command.payload.length != 0 || !playbackStarted
                            || state != P5cReceiverCoordinator.STATE_PAUSED
                            && state != P5cReceiverCoordinator.STATE_RECEIVER_RUNNING
                            && state != P5cReceiverCoordinator.STATE_STREAMING)
                        throw new IOException("invalid AOA RESUME state");
                    if (state == P5cReceiverCoordinator.STATE_PAUSED) playback.resume();
                    state = P5cReceiverCoordinator.STATE_RECEIVER_RUNNING;
                    break;
                case P5cControlProtocol.HEARTBEAT:
                    if (command.payload.length != 0) throw new IOException("invalid AOA HEARTBEAT");
                    if (playbackStarted && state == P5cReceiverCoordinator.STATE_RECEIVER_RUNNING && playback.streaming())
                        state = P5cReceiverCoordinator.STATE_STREAMING;
                    else if (state == P5cReceiverCoordinator.STATE_STREAMING && !playback.streaming())
                        state = P5cReceiverCoordinator.STATE_RECEIVER_RUNNING;
                    break;
                case P5cControlProtocol.PLAYBACK_PROGRESS_QUERY:
                    if (command.payload.length != 0) throw new IOException("invalid AOA progress query");
                    extra = playback.playbackProgressPayloadV2();
                    break;
                case P5cControlProtocol.STOP:
                    if (command.payload.length == 1 && command.payload[0] == 1) {
                        if ((supportedCapabilities & P5cControlProtocol.CAP_AOA_SESSION_RESTART) == 0)
                            throw new IOException("AOA restart capability was not negotiated");
                        restartRequested = true;
                    } else if (command.payload.length != 0) throw new IOException("invalid AOA STOP");
                    else hostStopped = true;
                    state = P5cReceiverCoordinator.STATE_STOPPED;
                    terminate = true;
                    break;
                default: throw new IOException("unsupported AOA control command");
            }
            byte[] ack = P5cControlProtocol.encode(keysSnapshot.receiverToHostControl,
                    P5cControlProtocol.RECEIVER_TO_HOST, sessionId, ackSequence++, P5cControlProtocol.ACK,
                    P5cControlProtocol.ackPayload(command.type, command.sequence,
                            result, state, port, extra));
            writeRecord(AnpuRecordCodec.CONTROL, ack);
            if (terminate) {
                stopAfterAck = true;
                if (playbackStarted) { playback.stop(); playbackStarted = false; }
                AoaSessionContext context = audio;
                if (context != null) context.stopQueue();
                cancel("AoaRemoteStop");
                return;
            }
        }
        throw new IOException("AOA session cancelled");
    }

    private void writeLoop() {
        try {
            while (!cancelled) {
                WriteRequest request = writes.poll(100, TimeUnit.MILLISECONDS);
                if (request == null) continue;
                try { io.writeRecord(request.type, request.body); request.done.countDown(); }
                catch (IOException error) { request.error = error; request.done.countDown(); throw error; }
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
        } catch (IOException error) { fail(error); }
    }

    private void writeRecord(int type, byte[] body) throws IOException {
        WriteRequest request = new WriteRequest(type, body);
        if (!writes.offer(request)) throw new IOException("AOA control write mailbox is full");
        try {
            if (!request.done.await(WRITE_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS))
                throw new IOException("AOA control write deadline expired");
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new IOException("AOA control write interrupted", interrupted);
        }
        if (request.error != null) throw request.error;
        if (cancelled && !stopAfterAck) throw new IOException("AOA write cancelled");
    }

    private AnpuRecordCodec.Record readExpected(int type) throws IOException {
        AnpuRecordCodec.Record record = io.readRecord();
        if (record.type != type) throw new IOException("unexpected AOA record during handshake");
        return record;
    }

    private void watchdogLoop() {
        while (!cancelled) {
            long now = System.nanoTime();
            if (!authenticated ? now >= startupDeadlineNanos
                    : now - lastControlAtNanos >= HEARTBEAT_TIMEOUT_NANOS) {
                fail(new IOException(authenticated ? "AOA heartbeat timeout" : "AOA startup timeout"));
                return;
            }
            try { Thread.sleep(100); }
            catch (InterruptedException interrupted) { return; }
        }
    }

    private void fail(IOException error) {
        synchronized (lifecycle) {
            if (cancelled) return;
            if (failure == null) failure = error;
        }
        AoaSessionContext context = audio;
        if (context != null) context.ingress.fail(error);
        cancel("AoaSessionFailure");
    }

    private void cleanup() {
        if (Thread.currentThread() == reader || Thread.currentThread() == writer) return;
        join(reader);
        join(writer);
        join(watchdog);
        boolean transportClosed = true;
        if (playbackStarted) {
            try { playback.stop(); }
            catch (RuntimeException error) { cleanupFailure = new IOException("AOA playback stop failed", error); }
            playbackStarted = false;
        }
        boolean interrupted = false;
        for (;;) try { io.awaitClosed(); break; }
        catch (InterruptedException error) { interrupted = true; }
        catch (IOException error) { cleanupFailure = combine(cleanupFailure, error); transportClosed = false; break; }
        AoaSessionContext context = audio;
        if (context != null) context.destroy();
        PairingProtocol.SessionKeys sessionKeys = keys;
        if (sessionKeys != null) sessionKeys.close();
        binding.close();
        if (transportClosed) synchronized (OWNER_LOCK) { if (owner == this) owner = null; }
        terminated.countDown();
        if (interrupted) Thread.currentThread().interrupt();
    }

    private static IOException combine(IOException first, IOException next) {
        if (first == null) return next;
        first.addSuppressed(next);
        return first;
    }

    private static void join(Thread thread) {
        if (thread == Thread.currentThread()) return;
        boolean interrupted = false;
        while (thread.isAlive()) try { thread.join(); }
        catch (InterruptedException error) { interrupted = true; }
        if (interrupted) Thread.currentThread().interrupt();
    }

    private static int readInt(byte[] bytes, int offset) {
        return (bytes[offset] << 24) | ((bytes[offset + 1] & 0xff) << 16)
                | ((bytes[offset + 2] & 0xff) << 8) | (bytes[offset + 3] & 0xff);
    }

    private static final class WriteRequest {
        final int type;
        final byte[] body;
        final CountDownLatch done = new CountDownLatch(1);
        volatile IOException error;
        WriteRequest(int type, byte[] body) { this.type = type; this.body = body; }
    }
}
