package app.anpod.receiver;

import java.io.IOException;
import java.io.InputStream;
import java.util.Arrays;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

/** Test-only Java endpoint used by Rust's merged P5-C localhost check. */
public final class P5cLoopbackEndpoint {
    public static void main(String[] args) throws Exception {
        PairingProtocol.BindingMaterial binding = new PairingProtocol.BindingMaterial(
                repeated((byte) 0x11, 16), repeated((byte) 0x22, 16), repeated((byte) 0x33, 32));
        FakePlayback playback = new FakePlayback();
        try (P5cReceiverCoordinator endpoint = P5cReceiverCoordinator.openLoopback(binding, playback)) {
            endpoint.start();
            System.out.println("PORT=" + endpoint.port());
            System.out.flush();
            if (!endpoint.await(15, TimeUnit.SECONDS)) throw new IOException("P5-C test endpoint timed out");
            System.out.println("RESULT=" + endpoint.failure() + "," + playback.starts.get() + ","
                    + playback.pauses.get() + "," + playback.resumes.get() + "," + playback.stops.get() + ","
                    + playback.pcmBytes.get() + "," + playback.udpClosed.get());
            System.out.flush();
        }
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
        volatile UdpPcmQueue queue;
        volatile UdpReceiver receiver;
        volatile Thread receiverThread;
        volatile Thread sinkThread;

        @Override public boolean start(UdpSessionContext context) throws IOException {
            starts.incrementAndGet();
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
        @Override public void resume() { resumes.incrementAndGet(); if (queue != null) queue.setPaused(false); }
        @Override public boolean streaming() { return streaming.get(); }

        @Override public void stop() {
            if (!stopped.compareAndSet(false, true)) return;
            stops.incrementAndGet();
            UdpReceiver currentReceiver = receiver;
            if (currentReceiver != null) currentReceiver.close();
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
