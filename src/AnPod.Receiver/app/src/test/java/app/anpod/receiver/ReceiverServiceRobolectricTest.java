package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import android.content.Intent;
import android.media.AudioFormat;
import android.media.AudioTrack;
import java.io.IOException;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.atomic.AtomicBoolean;
import java.lang.reflect.Field;

import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.Robolectric;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.android.controller.ServiceController;
import org.robolectric.shadows.ShadowAudioTrack;
import org.robolectric.annotation.Config;
import org.robolectric.annotation.Implementation;
import org.robolectric.annotation.Implements;

/** JVM lifecycle and snapshot checks for the foreground service; no device is involved. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 34, shadows = ReceiverServiceRobolectricTest.FloatWriteShadow.class)
public final class ReceiverServiceRobolectricTest {
    private static final AtomicReference<float[]> LAST_FLOAT_WRITE = new AtomicReference<>();

    @Implements(value = AudioTrack.class)
    public static final class FloatWriteShadow extends ShadowAudioTrack {
        @Implementation
        protected int write(float[] audioData, int offsetInFloats, int sizeInFloats, int writeMode) {
            float[] captured = new float[sizeInFloats];
            System.arraycopy(audioData, offsetInFloats, captured, 0, sizeInFloats);
            LAST_FLOAT_WRITE.set(captured);
            return sizeInFloats;
        }
    }

    private static final AtomicBoolean MISMATCH_TRACK_RELEASED = new AtomicBoolean();

    @Implements(value = AudioTrack.class)
    public static final class WrongSampleRateShadow extends ShadowAudioTrack {
        @Implementation
        protected int getSampleRate() { return 48_000; }

        @Implementation
        protected void release() {
            MISMATCH_TRACK_RELEASED.set(true);
        }
    }

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
        assertEquals(BuildConfig.VERSION_CODE, snapshot.versionCode);
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
    public void floatPartialWriteStopsWhenPauseChangesTheBlockGeneration() throws Exception {
        AudioFocusState focus = new AudioFocusState();
        long generation = focus.writeGeneration();
        int[] writeCalls = {0};

        boolean completed = ReceiverService.writeV2WithGeneration(focus, generation, 5,
                (offsetFrames, remainingFrames) -> {
                    writeCalls[0]++;
                    assertEquals(0, offsetFrames);
                    assertEquals(5, remainingFrames);
                    focus.setUserPaused(true);
                    return 2;
                });

        assertFalse(completed);
        assertEquals(1, writeCalls[0]);
    }

    @Test
    public void floatPreRollRestartsAfterPauseFlushAndReadToWriteRace() throws Exception {
        ReceiverService.V2PreRollProgress progress = new ReceiverService.V2PreRollProgress(1);
        progress.recordWrite(1, 1, 240, true);
        progress.recordWrite(1, 1, 240, true);
        assertEquals(480, progress.completedFrames());

        // The queue read began in generation 1, while pause, flush, and resume happened before write.
        progress.recordWrite(1, 3, 240, false);
        assertEquals(0, progress.completedFrames());
        assertEquals(3, progress.generation());

        progress.observeGeneration(3);
        progress.recordWrite(3, 3, 240, true);
        progress.recordWrite(3, 3, 240, true);
        progress.recordWrite(3, 3, 240, true);
        assertEquals(720, progress.completedFrames());
    }

    @Test
    public void packed24FailureSelectsFloatAndReportsBothFailures() throws Exception {
        int[] attempts = {0};
        ReceiverService.OpenedV2Track opened = ReceiverService.chooseV2AudioTrack(34, 48_000,
                AudioFormat.CHANNEL_OUT_STEREO, (sampleRate, channelMask, encoding, formatCode) -> {
                    attempts[0]++;
                    if (encoding == AudioFormat.ENCODING_PCM_24BIT_PACKED)
                        throw new IOException("packed unsupported");
                    assertEquals(AudioFormat.ENCODING_PCM_FLOAT, encoding);
                    return new ReceiverService.OpenedV2Track(null, formatCode, 1, 8);
                });
        assertEquals(2, attempts[0]);
        assertEquals(3, opened.formatCode);

        try {
            ReceiverService.chooseV2AudioTrack(34, 48_000, AudioFormat.CHANNEL_OUT_STEREO,
                    (sampleRate, channelMask, encoding, formatCode) -> {
                        throw new IOException("encoding " + encoding + " unsupported");
                    });
            throw new AssertionError("unsupported packed24 and Float32 tracks were accepted");
        } catch (IOException expected) {
            assertEquals("Neither packed PCM24 nor Float32 AudioTrack is supported", expected.getMessage());
            assertEquals(1, expected.getSuppressed().length);
        }
    }

    @Test
    public void v2PolicyAcceptsOnlyTheExact48And96KhzPacked24Profiles() {
        assertTrue(ReceiverService.supportsPcm24AudioFormatV2(
                UdpPacketDecoder.AudioFormatV2.forProfile(2, 0)));
        assertTrue(ReceiverService.supportsPcm24AudioFormatV2(
                UdpPacketDecoder.AudioFormatV2.forProfile(3, 1)));
        assertFalse(ReceiverService.supportsPcm24AudioFormatV2(
                UdpPacketDecoder.AudioFormatV2.forProfile(1, 0)));
    }

    @Test
    public void phase4OpensAtTheRequested96KhzOrFailsInsteadOfChangingRate() throws Exception {
        ReceiverService.OpenedV2Track opened = ReceiverService.openV2AudioTrack(
                96_000, AudioFormat.CHANNEL_OUT_STEREO);
        try {
            assertEquals(96_000, opened.track.getSampleRate());
            assertEquals(2, opened.track.getChannelCount());
            assertEquals(opened.formatCode == 2 ? AudioFormat.ENCODING_PCM_24BIT_PACKED
                    : AudioFormat.ENCODING_PCM_FLOAT, opened.track.getAudioFormat());
        } finally {
            opened.track.release();
        }

        int[] attemptedRate = {0};
        ReceiverService.OpenedV2Track fallback = ReceiverService.chooseV2AudioTrack(30, 96_000,
                AudioFormat.CHANNEL_OUT_STEREO, (sampleRate, channelMask, encoding, formatCode) -> {
                    attemptedRate[0] = sampleRate;
                    assertEquals(AudioFormat.ENCODING_PCM_FLOAT, encoding);
                    return new ReceiverService.OpenedV2Track(null, formatCode, 1, 8);
                });
        assertEquals(96_000, attemptedRate[0]);
        assertEquals(3, fallback.formatCode);
    }

    @Test
    @Config(sdk = 34, shadows = WrongSampleRateShadow.class)
    public void phase4RejectsAndReleasesA96KhzTrackOpenedAt48Khz() throws Exception {
        MISMATCH_TRACK_RELEASED.set(false);
        try {
            ReceiverService.openV2AudioTrack(96_000, AudioFormat.CHANNEL_OUT_STEREO);
            throw new AssertionError("AudioTrack silently opened 96k PCM at 48k");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("Neither packed PCM24 nor Float32"));
        }
        assertTrue("mismatched AudioTracks must be released", MISMATCH_TRACK_RELEASED.get());
        MISMATCH_TRACK_RELEASED.set(false);
    }

    @Test
    public void v2WorkerWritesOpenedHighResolutionFormatWithoutPcm16Quantization() throws Exception {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(42,
                new ReceiverService.AudioRequest(null, null, "balanced"), "test-owner", 1);
        ReceiverService.currentAudio = session;
        AtomicReference<byte[]> writtenData = new AtomicReference<>();
        ShadowAudioTrack.OnAudioDataWrittenListener listener = (shadow, data, format) ->
                writtenData.set(data.clone());
        ShadowAudioTrack.addAudioDataListener(listener);
        ReceiverService.OpenedV2Track opened = null;
        try {
            opened = ReceiverService.openV2AudioTrack(48_000, AudioFormat.CHANNEL_OUT_STEREO);
            AudioTrack track = opened.track;
            assertEquals(2, track.getChannelCount());
            assertEquals(48_000, track.getSampleRate());
            assertEquals(opened.formatCode == 2 ? AudioFormat.ENCODING_PCM_24BIT_PACKED
                    : AudioFormat.ENCODING_PCM_FLOAT, track.getAudioFormat());
            track.play();

            float lsb = 1f / 8_388_608f;
            float[] samples = {lsb, -lsb, 0.5f, -0.5f};
            byte[] packed = opened.formatCode == 2 ? new byte[opened.frameBytes * 2] : null;
            assertTrue(service.writeV2Frames(session, track, opened, samples, 2, packed,
                    session.focusState.writeGeneration()));

            if (opened.formatCode == 2) {
                byte[] bytes = writtenData.get();
                assertNotNull(bytes);
                assertArrayEquals(new byte[] {1, 0, 0, -1, -1, -1}, java.util.Arrays.copyOf(bytes, 6));
            } else {
                float[] output = LAST_FLOAT_WRITE.get();
                assertNotNull(output);
                assertEquals(lsb, output[0], 0f);
                assertEquals(-lsb, output[1], 0f);
            }
        } finally {
            ShadowAudioTrack.removeAudioDataListener(listener);
            LAST_FLOAT_WRITE.set(null);
            if (opened != null) opened.track.release();
            ReceiverService.currentAudio = null;
            service.onDestroy();
        }
    }


    @Test
    public void binderReturnsTheServiceSnapshot() {
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverUiBinder binder = (ReceiverUiBinder) service.onBind(new Intent());

        UiSnapshot snapshot = binder.snapshot();
        assertEquals(BuildConfig.VERSION_CODE, snapshot.versionCode);
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
