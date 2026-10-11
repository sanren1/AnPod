package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.util.Arrays;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

/** Test-only Java endpoint used by Rust's merged P5-C localhost check. */
public final class P5cLoopbackEndpoint {
    public static void main(String[] args) throws Exception {
        verifyFragmentedControlFrame();
        verifyHeartbeatDeadline();
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        FakePlayback playback = new FakePlayback(Arrays.asList(args).contains("--fail-resume"));
        P5cReceiverCoordinator opened = Arrays.asList(args).contains("--discovery")
                ? P5cReceiverCoordinator.openOnInterface(binding, playback, InetAddress.getByName("127.0.0.1"))
                : P5cReceiverCoordinator.openLoopback(binding, playback);
        try (P5cReceiverCoordinator endpoint = opened) {
            endpoint.start();
            if (Arrays.asList(args).contains("--discovery")) {
                endpoint.startDiscovery();
                verifyInvalidDiscoveryDoesNotStartPlayback(playback);
            }
            System.out.println("PORT=" + endpoint.port());
            System.out.flush();
            if (args.length > 0 && "--drop-on-stdin".equals(args[0])) {
                if (System.in.read() < 0) throw new IOException("P5-D test disconnect signal was not supplied");
                endpoint.close();
            }
            if (!endpoint.await(15, TimeUnit.SECONDS)) throw new IOException("P5-C test endpoint timed out");
            System.out.println("RESULT=" + endpoint.failure() + "," + playback.starts.get() + ","
                    + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get() + ","
                    + playback.pcmBytes.get() + "," + playback.udpClosed.get());
            System.out.flush();
        }
    }

    private static void verifyInvalidDiscoveryDoesNotStartPlayback(FakePlayback playback) throws IOException {
        try (DatagramSocket probe = new DatagramSocket(0, InetAddress.getByName("127.0.0.1"))) {
            probe.setSoTimeout(100);
            byte[] malformed = new byte[26];
            probe.send(new DatagramPacket(malformed, malformed.length,
                    new InetSocketAddress(InetAddress.getByName("127.0.0.1"), P5cDiscovery.PORT)));
            try {
                probe.receive(new DatagramPacket(new byte[64], 64));
                throw new AssertionError("malformed discovery probe received a response");
            } catch (SocketTimeoutException expected) { }
        }
        if (playback.starts.get() != 0) throw new AssertionError("discovery started playback before authentication");
    }

    private static void verifyFragmentedControlFrame() throws Exception {
        byte[] key = new byte[32];
        byte[] frame = P5cControlProtocol.encode(key, P5cControlProtocol.HOST_TO_RECEIVER,
                7, 0, P5cControlProtocol.HEARTBEAT, new byte[0]);
        AtomicReference<Throwable> failure = new AtomicReference<>();
        try (ServerSocket listener = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
            Thread reader = new Thread(() -> {
                try (Socket socket = listener.accept()) {
                    socket.setSoTimeout(25);
                    P5cControlProtocol.Frame parsed = P5cControlProtocol.read(
                            new DataInputStream(socket.getInputStream()), key,
                            P5cControlProtocol.HOST_TO_RECEIVER, 7);
                    if (parsed.type != P5cControlProtocol.HEARTBEAT || parsed.sequence != 0)
                        throw new AssertionError("fragmented heartbeat changed");
                } catch (Throwable error) { failure.set(error); }
            }, "P5-C fragmented-frame parser check");
            reader.start();
            try (Socket client = new Socket(InetAddress.getByName("127.0.0.1"), listener.getLocalPort())) {
                DataOutputStream output = new DataOutputStream(client.getOutputStream());
                output.writeByte(frame.length >>> 8);
                output.flush();
                Thread.sleep(100);
                output.writeByte(frame.length);
                output.write(frame);
                output.flush();
            }
            reader.join(2_000);
            if (reader.isAlive()) throw new AssertionError("fragmented-frame parser did not finish");
        }
        if (failure.get() != null) throw new AssertionError("fragmented-frame parser failed", failure.get());
    }

    private static void verifyHeartbeatDeadline() throws Exception {
        P5cReceiverCoordinator.checkHeartbeatDeadline(5_000_000_000L, 0);
        try {
            P5cReceiverCoordinator.checkHeartbeatDeadline(5_000_000_001L, 0);
            throw new AssertionError("expired authenticated heartbeat was accepted");
        } catch (IOException expected) { }
    }

    private static byte[] repeated(byte value, int length) {
        byte[] bytes = new byte[length];
        Arrays.fill(bytes, value);
        return bytes;
    }

    private static final class FakePlayback implements P5cReceiverCoordinator.PlaybackEndpoint {
        final AtomicInteger starts = new AtomicInteger();
        final AtomicInteger pauses = new AtomicInteger();
        final AtomicInteger resumes = new AtomicInteger();
        final AtomicInteger stops = new AtomicInteger();
        final AtomicInteger pcmBytes = new AtomicInteger();
        final AtomicBoolean streaming = new AtomicBoolean();
        final AtomicBoolean udpClosed = new AtomicBoolean();
        final AtomicBoolean stopped = new AtomicBoolean();
        final AtomicBoolean failResume;
        volatile UdpPcmQueue queue;
        volatile UdpReceiver receiver;
        volatile UdpSessionContext context;
        volatile Thread receiverThread;
        volatile Thread sinkThread;

        FakePlayback(boolean failResume) { this.failResume = new AtomicBoolean(failResume); }

        @Override public boolean start(UdpSessionContext context) throws IOException {
            starts.incrementAndGet();
            this.context = context;
            queue = new UdpPcmQueue(context.parameters);
            receiver = new UdpReceiver(context, queue);
            receiverThread = new Thread(receiver, "P5-C test UDP receiver");
            receiverThread.start();
            InputStream pcm = queue.asPcmInputStream();
            sinkThread = new Thread(() -> {
                try (pcm) {
                    byte[] header = new byte[8];
                    readFully(pcm, header);
                    byte[] buffer = new byte[1024];
                    int count;
                    while ((count = pcm.read(buffer)) >= 0) {
                        if (count != 0) {
                            pcmBytes.addAndGet(count);
                            streaming.set(true);
                        }
                    }
                } catch (IOException ignored) { }
            }, "P5-C test synthetic PCM sink");
            sinkThread.start();
            return true;
        }

        @Override public void pause() { pauses.incrementAndGet(); if (queue != null) queue.setPaused(true); }
        @Override public void resume() throws IOException {
            resumes.incrementAndGet();
            if (failResume.compareAndSet(true, false)) throw new IOException("P5-D test RESUME failure");
            if (queue != null) queue.setPaused(false);
        }
        @Override public boolean streaming() { return streaming.get(); }

        @Override public void stop() {
            if (!stopped.compareAndSet(false, true)) return;
            stops.incrementAndGet();
            UdpReceiver currentReceiver = receiver;
            if (currentReceiver != null) currentReceiver.close();
            UdpSessionContext currentContext = context;
            if (currentContext != null) currentContext.destroy();
            UdpPcmQueue currentQueue = queue;
            if (currentQueue != null) currentQueue.stop();
            join(receiverThread);
            join(sinkThread);
            udpClosed.set(currentReceiver == null || currentReceiver.isClosed());
        }

        private static void join(Thread thread) {
            if (thread == null || thread == Thread.currentThread()) return;
            try { thread.join(1_000); }
            catch (InterruptedException error) { Thread.currentThread().interrupt(); }
        }

        private static void readFully(InputStream input, byte[] bytes) throws IOException {
            int offset = 0;
            while (offset < bytes.length) {
                int count = input.read(bytes, offset, bytes.length - offset);
                if (count < 0) throw new IOException("truncated PCM header");
                offset += count;
            }
        }
    }
}
