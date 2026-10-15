package app.anpod.receiver;

import android.app.ActivityManager;
import android.content.Context;
import android.content.Intent;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Build;
import android.test.InstrumentationTestCase;

import java.io.DataInputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.UUID;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;

/** Device-side lifecycle smoke entry; it sends silence and is not run by host builds. */
public final class ReceiverServiceLifecycleTest extends InstrumentationTestCase {
    private static final String TOKEN = "anpod-lifecycle-check";
    private Context context;

    @Override
    protected void setUp() throws Exception {
        super.setUp();
        context = getInstrumentation().getTargetContext();
    }

    public void testStopEofFailureAndRapidReconnect() throws Exception {
        String stoppedName = socketName();
        try (LocalServerSocket server = new LocalServerSocket(stoppedName);
                LocalSocket peer = acceptAndReadToken(server, stoppedName)) {
            context.stopService(new Intent(context, ReceiverService.class));
            assertFalse("stop must clear reported playback immediately", ReceiverService.audioActive);
            assertServiceStopped();
        }

        String eofName = socketName();
        try (LocalServerSocket server = new LocalServerSocket(eofName);
                LocalSocket peer = acceptAndReadToken(server, eofName)) {
            peer.getOutputStream().write(pcmHeader());
            peer.getOutputStream().flush();
            peer.shutdownOutput();
            assertServiceStopped();
            assertTrue("EOF during prebuffer must remain diagnosable", ReceiverService.startFailure.contains("AudioStreamEndedBeforePlayback"));
        }

        String oldName = socketName();
        String newName = socketName();
        try (LocalServerSocket oldServer = new LocalServerSocket(oldName);
                LocalServerSocket newServer = new LocalServerSocket(newName);
                LocalSocket oldPeer = acceptAndReadToken(oldServer, oldName)) {
            startAudio(newName);
            try (LocalSocket newPeer = acceptConnection(newServer)) {
                newPeer.getOutputStream().write(new byte[8]);
                newPeer.getOutputStream().flush();
                assertServiceStopped();
                assertTrue("replacement session failure must be reported", ReceiverService.startFailure.contains("Unsupported PCM format"));
            }
        }
    }

    public void testUiBootstrapGetsBoundedPreparationWindowAndAdbKeepsShortTimeout() {
        assertEquals(120_000L, ReceiverService.networkBootstrapTimeoutMs(true));
        assertEquals(25_000L, ReceiverService.networkBootstrapTimeoutMs(false));
    }

    private LocalSocket acceptAndReadToken(LocalServerSocket server, String socketName) throws Exception {
        startAudio(socketName);
        return acceptConnection(server);
    }

    private LocalSocket acceptConnection(LocalServerSocket server) throws Exception {
        FutureTask<LocalSocket> accept = new FutureTask<>(() -> {
            return server.accept();
        });
        Thread thread = new Thread(accept, "Receiver lifecycle test accept");
        thread.start();
        LocalSocket peer = accept.get(5, TimeUnit.SECONDS);
        byte[] token = new byte[TOKEN.length()];
        new DataInputStream(peer.getInputStream()).readFully(token);
        assertEquals(TOKEN, new String(token, java.nio.charset.StandardCharsets.UTF_8));
        return peer;
    }

    private void startAudio(String socketName) {
        Intent intent = new Intent(context, ReceiverService.class)
                .setAction(BootstrapActivity.ACTION_START_AUDIO)
                .putExtra("socketName", socketName)
                .putExtra("connectCode", TOKEN)
                .putExtra("playbackMode", "balanced");
        if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent);
        else context.startService(intent);
    }

    private void assertServiceStopped() throws Exception {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
        while (System.nanoTime() < deadline) {
            ActivityManager manager = context.getSystemService(ActivityManager.class);
            boolean running = manager.getRunningServices(Integer.MAX_VALUE).stream()
                    .anyMatch(service -> ReceiverService.class.getName().equals(service.service.getClassName()) && service.started);
            if (!running) return;
            Thread.sleep(50);
        }
        fail("Receiver service did not stop within five seconds");
    }

    private static String socketName() { return "anpod_test_" + UUID.randomUUID().toString().replace("-", ""); }

    private static byte[] pcmHeader() {
        return ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)
                .putInt(8000).putShort((short) 1).putShort((short) 16).array();
    }
}
