package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;

import android.content.Intent;
import java.lang.reflect.Field;

import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.Robolectric;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.android.controller.ServiceController;
import org.robolectric.annotation.Config;

/** JVM lifecycle and snapshot checks for the foreground service; no device is involved. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 34)
public final class ReceiverServiceRobolectricTest {
    @Test
    public void serviceRegistersItselfAndClearsOnDestroy() throws Exception {
        ServiceController<ReceiverService> controller = Robolectric.buildService(ReceiverService.class);
        ReceiverService service = controller.create().get();

        assertNotNull(service);
        assertSame(service, activeService());

        controller.destroy();
        assertNull(activeService());
    }

    @Test
    public void uiSnapshotDescribesTheIdleService() throws Exception {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        UiSnapshot snapshot = invokeSnapshot(service);

        assertNotNull(snapshot.serviceState);
        assertEquals(15, snapshot.versionCode);
        assertNotNull(snapshot.searchCandidates);
        assertEquals(false, snapshot.searchActive);
    }

    @Test
    public void playbackMetricsAreUnknownWithoutAnAudioSession() {
        PlaybackMetrics metrics = ReceiverService.playbackMetricsSnapshot();

        assertEquals(-1, metrics.sampleRate);
        assertEquals(-1, metrics.channels);
    }


    @Test
    public void binderReturnsTheServiceSnapshot() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverUiBinder binder = (ReceiverUiBinder) service.onBind(new Intent());

        UiSnapshot snapshot = binder.snapshot();
        assertEquals(15, snapshot.versionCode);
        assertNotNull(snapshot.serviceState);
        assertNotNull(binder.interfaces());
    }

    @Test
    public void malformedAuthorizationDecisionIsIgnored() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();

        service.authorization.handleDecision(new Intent(), 1);
        service.authorization.cancel();
    }

    @Test
    public void idleStopIsRejected() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverUiBinder binder = (ReceiverUiBinder) service.onBind(new Intent());

        assertFalse(binder.stop("unknown-owner"));
        assertFalse(service.stopFromUi("unknown-owner", 1L));
    }

    @Test
    public void aodSnapshotIsDisconnectedWhenIdle() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();

        AodTelemetrySnapshot snapshot = service.uiState.aodSnapshot();
        assertEquals(AodTelemetrySnapshot.DisplayStatus.DISCONNECTED, snapshot.status);
        assertNull(snapshot.recvAudioPayloadBps);
        assertNull(snapshot.receiverBufferMs);
        assertNull(snapshot.udpLossPercent);
        assertFalse(snapshot.receivedAudioRecently);
    }


    @Test
    public void idleWaveformIsFlat() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverUiBinder binder = (ReceiverUiBinder) service.onBind(new Intent());

        PcmWaveformTap.Waveform waveform = binder.aodWaveform();
        for (float value : waveform.left) assertEquals(0f, value, 0.0001f);
        for (float value : waveform.right) assertEquals(0f, value, 0.0001f);
    }

    private static UiSnapshot invokeSnapshot(ReceiverService service) {
        return service.uiState.snapshot();
    }

    private static ReceiverService activeService() throws Exception {
        Field field = ReceiverService.class.getDeclaredField("activeService");
        field.setAccessible(true);
        return (ReceiverService) field.get(null);
    }
}
