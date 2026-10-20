package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import android.content.Intent;
import android.media.AudioManager;
import android.media.AudioFormat;
import android.media.AudioTrack;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.atomic.AtomicBoolean;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;

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

    private static final class FakeOboeCalls implements OboeNativeBridge.NativeCalls {
        int openCalls, startCalls, closeCalls, closeFailures;
        int sampleRate, channels, budgetFrames, ringFrames, acceptedFrames;
        int openStatus, startStatus, zeroWrites, pauseAndFlushStatus;
        Runnable onZeroWrite;
        Runnable onPauseRequest;
        Runnable onPauseAndFlush;
        Runnable onSetVolume;
        Runnable onGetMetrics;
        int metricsStatus;
        int pauseAndFlushCalls;
        int reportedHead = 0x80000000;
        final List<Integer> offsets = new ArrayList<>();
        final List<Float> samples = new ArrayList<>();
        int acceptedAtStart = -1;

        @Override public long open(int rate, int channelCount, int budget, int requestedRing, int[] info) {
            openCalls++;
            sampleRate = rate;
            channels = channelCount;
            budgetFrames = budget;
            ringFrames = Math.min(requestedRing, budget - 240);
            info[0] = openStatus; info[1] = rate; info[2] = channelCount; info[3] = 1;
            info[4] = 48; info[5] = 240; info[6] = 960; info[7] = ringFrames;
            info[8] = 1; info[9] = 1; info[10] = 3; info[11] = budget;
            return 7;
        }
        @Override public int writeFloat(long handle, float[] data, int offsetFrames, int frameCount) {
            offsets.add(offsetFrames);
            if (zeroWrites > 0) {
                zeroWrites--;
                if (onZeroWrite != null) onZeroWrite.run();
                return 0;
            }
            int accepted = Math.min(2, frameCount);
            for (int i = 0; i < accepted * channels; i++)
                samples.add(data[offsetFrames * channels + i]);
            acceptedFrames += accepted;
            return accepted;
        }
        @Override public int requestPause(long handle) {
            if (onPauseRequest != null) onPauseRequest.run();
            return 0;
        }
        @Override public int pauseAndFlush(long handle) {
            pauseAndFlushCalls++;
            if (onPauseAndFlush != null) onPauseAndFlush.run();
            if (pauseAndFlushStatus == 0) acceptedFrames = 0;
            return pauseAndFlushStatus;
        }
        @Override public int start(long handle, int[] info) {
            startCalls++;
            acceptedAtStart = acceptedFrames;
            return startStatus;
        }
        @Override public int setVolume(long handle, float volume) {
            if (onSetVolume != null) onSetVolume.run();
            return 0;
        }
        @Override public int getMetrics(long handle, int[] metrics, long[] timestamp) {
            if (onGetMetrics != null) onGetMetrics.run();
            metrics[0] = acceptedFrames;
            metrics[1] = acceptedFrames;
            metrics[4] = 0; metrics[5] = 0; metrics[6] = reportedHead; metrics[7] = reportedHead;
            metrics[8] = 13; metrics[10] = 0;
            timestamp[0] = -1; timestamp[1] = -1;
            return metricsStatus;
        }
        @Override public int close(long handle) {
            closeCalls++;
            if (closeFailures > 0) { closeFailures--; return OboeNativeBridge.ERR_STREAM_FAILED; }
            return 0;
        }
        @Override public String lastErrorMessage() { return "fake Oboe error"; }
    }

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
    public void oboeFactoryPartialWritesKeepFloatPrecisionPrefillAndUnsignedApiProgress() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(84,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        session.requestedOutputEngine = "oboeExperimental";
        session.actualOutputEngine = "unknown";
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            assertTrue(output instanceof OboePlaybackOutput);
            assertEquals("oboeExperimental", session.actualOutputEngine);
            assertEquals(240, output.ringCapacityFrames());
            assertTrue(output.ringCapacityFrames() >= output.framesPerBurst());

            float lsb = 1f / 8_388_608f;
            float[] samples = {lsb, -lsb, 0.25f, -0.25f, 0.5f, -0.5f, 0.75f, -0.75f};
            assertTrue(service.writeV2Frames(session, output, samples, 4, null,
                    session.focusState.writeGeneration()));
            assertEquals(java.util.Arrays.asList(0, 2), nativeCalls.offsets);
            assertEquals(lsb, nativeCalls.samples.get(0), 0f);
            assertEquals(-lsb, nativeCalls.samples.get(1), 0f);
            output.play();
            assertEquals("initial start must keep prefilled ring frames", 4, nativeCalls.acceptedAtStart);
            assertEquals(2_147_483_648L, output.playbackHeadPosition());
            nativeCalls.reportedHead = -1;
            assertEquals(4_294_967_295L, output.playbackHeadPosition());
            assertEquals(-1, output.underrunCount());
            assertEquals(13, output.oboeXrunCount());

            byte[] pcm16 = {1, 0, -1, -1};
            assertEquals(4, output.writePcm16(pcm16, 0, pcm16.length));
            assertEquals(1f / 32768f, nativeCalls.samples.get(8), 0f);
            assertEquals(-1f / 32768f, nativeCalls.samples.get(9), 0f);
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void oboeZeroWriteStopsOnGenerationChangeAndFlushStartsANewCounterEpoch() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(88,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        session.requestedOutputEngine = "oboeExperimental";
        session.actualOutputEngine = "unknown";
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            long generation = session.focusState.writeGeneration();
            nativeCalls.zeroWrites = 1;
            nativeCalls.onZeroWrite = () -> session.focusState.setUserPaused(true);
            float[] samples = {0.1f, -0.1f, 0.2f, -0.2f};
            assertFalse(service.writeV2Frames(session, output, samples, 2, null, generation));
            assertEquals(1, nativeCalls.offsets.size());
            assertEquals(0, nativeCalls.acceptedFrames);

            output.requestPause();
            output.pauseAndFlush();
            session.focusState.setUserPaused(false);
            nativeCalls.onZeroWrite = null;
            output.play();
            long resumedGeneration = session.focusState.writeGeneration();
            assertTrue(service.writeV2Frames(session, output, samples, 2, null, resumedGeneration));
            output.metrics();
            assertEquals("the flush starts a fresh metric epoch", 2, ((OboePlaybackOutput) output).extendedCounter(0));
            assertEquals(2, nativeCalls.acceptedFrames);
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void userStopSerializesOboePauseBeforeWorkerCanCloseOutput() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(93,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        session.requestedOutputEngine = "oboeExperimental";
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        boolean[] pauseHeldSessionLock = {false};
        String pendingBefore = ReceiverService.pendingOboeFallbackReasonForUi();
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            assertNotNull(output);
            nativeCalls.onPauseRequest = () -> pauseHeldSessionLock[0] = Thread.holdsLock(session);

            // onDestroy follows the production stop path; worker cleanup also owns this monitor.
            service.onDestroy();

            assertTrue("stop must request Oboe mute under the session ownership lock",
                    pauseHeldSessionLock[0]);
            assertEquals(pendingBefore, ReceiverService.pendingOboeFallbackReasonForUi());
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void focusControlsSerializeMuteAndVolumeAndIgnoreStaleFlushErrors() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        String owner = "focus-control-test";
        assertTrue(service.uiOperation.begin(owner));
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(94,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), owner,
                service.currentOperationGeneration);
        session.requestedOutputEngine = "oboeExperimental";
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        boolean[] pauseHeldSessionLock = {false};
        boolean[] volumeHeldSessionLock = {false};
        String pendingBefore = ReceiverService.pendingOboeFallbackReasonForUi();
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            assertNotNull(output);
            PlaybackOutput openedOutput = output;
            nativeCalls.onPauseRequest = () -> pauseHeldSessionLock[0] = Thread.holdsLock(session);
            nativeCalls.onSetVolume = () -> volumeHeldSessionLock[0] = Thread.holdsLock(session);

            Method focusChange = ReceiverService.class.getDeclaredMethod("handleAudioFocusChange",
                    ReceiverService.AudioSession.class, PlaybackOutput.class, int.class);
            focusChange.setAccessible(true);
            focusChange.invoke(service, session, openedOutput,
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK);
            nativeCalls.onPauseAndFlush = () -> {
                session.cancelled = true;
                session.focusState.cancel();
                nativeCalls.pauseAndFlushStatus = OboeNativeBridge.ERR_STREAM_FAILED;
            };
            focusChange.invoke(service, session, openedOutput, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT);

            assertTrue("focus mute request must share worker close ownership", pauseHeldSessionLock[0]);
            assertTrue("focus volume changes must share worker close ownership", volumeHeldSessionLock[0]);
            assertEquals(1, nativeCalls.pauseAndFlushCalls);
            assertEquals(pendingBefore, ReceiverService.pendingOboeFallbackReasonForUi());
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void userPauseSerializesMuteAndIgnoresFlushFailureAfterStop() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        InetAddress loopback = InetAddress.getLoopbackAddress();
        UdpSessionContext context = new UdpSessionContext(
                new InetSocketAddress(loopback, 0), new InetSocketAddress(loopback, 12345),
                96, new byte[32], "lowlatency");
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(96,
                ReceiverService.AudioRequest.udp(context), "test-owner", 1);
        session.requestedOutputEngine = "oboeExperimental";
        session.trackReady = true;
        session.trackPlaying = true;
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        boolean[] pauseHeldSessionLock = {false};
        String pendingBefore = ReceiverService.pendingOboeFallbackReasonForUi();
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            assertNotNull(output);
            nativeCalls.onPauseRequest = () -> pauseHeldSessionLock[0] = Thread.holdsLock(session);
            nativeCalls.onPauseAndFlush = () -> {
                session.cancelled = true;
                session.focusState.cancel();
                nativeCalls.pauseAndFlushStatus = OboeNativeBridge.ERR_STREAM_FAILED;
            };

            Method userPause = ReceiverService.class.getDeclaredMethod("setUdpAudioPaused",
                    ReceiverService.AudioSession.class, boolean.class);
            userPause.setAccessible(true);
            userPause.invoke(service, session, true);

            assertTrue("user pause must mute under worker-close ownership", pauseHeldSessionLock[0]);
            assertEquals(1, nativeCalls.pauseAndFlushCalls);
            assertEquals("a stop during flush is not an Oboe engine failure", pendingBefore,
                    ReceiverService.pendingOboeFallbackReasonForUi());
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            context.destroy();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void streamingProgressFailureAfterStopDoesNotScheduleOboeFallback() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession session = new ReceiverService.AudioSession(95,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        session.requestedOutputEngine = "oboeExperimental";
        session.floatQueue = new com.anpod.server.FloatPcmFrameQueue(48_000, true);
        session.trackPlaying = true;
        ReceiverService.currentAudio = session;
        PlaybackOutput output = null;
        String pendingBefore = ReceiverService.pendingOboeFallbackReasonForUi();
        try {
            output = service.openOboeOutputIfRequested(session, 48_000, 2, true, 240);
            assertNotNull(output);
            nativeCalls.metricsStatus = OboeNativeBridge.ERR_STREAM_FAILED;
            nativeCalls.onGetMetrics = () -> {
                session.cancelled = true;
                session.focusState.cancel();
            };

            assertFalse(session.isStreamingAt(System.nanoTime()));
            assertEquals(pendingBefore, ReceiverService.pendingOboeFallbackReasonForUi());
        } finally {
            service.onDestroy();
            if (output != null) output.close();
            OboeNativeBridge.setTestCalls(null);
            resetWaveformForTest();
        }
    }

    @Test
    public void deferredOboeCloseBlocksNewNativeOpenUntilTheHandleCloses() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession first = new ReceiverService.AudioSession(85,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        first.requestedOutputEngine = "oboeExperimental";
        try {
            OboePlaybackOutput output = (OboePlaybackOutput) service.openOboeOutputIfRequested(
                    first, 48_000, 2, true, 240);
            nativeCalls.closeFailures = 3;
            try { output.close(); throw new AssertionError("first close should fail"); }
            catch (OboeOutputException expected) { ReceiverService.noteOboeRuntimeFailure(first, expected); }
            ReceiverService.retainDeferredOboeClose(output);
            try { output.close(); throw new AssertionError("second close should fail"); }
            catch (OboeOutputException expected) { }

            ReceiverService.AudioSession fallback = new ReceiverService.AudioSession(86,
                    new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
            fallback.requestedOutputEngine = "oboeExperimental";
            assertNull(service.openOboeOutputIfRequested(fallback, 48_000, 2, true, 240));
            assertEquals(1, nativeCalls.openCalls);
            assertFalse(fallback.outputFallbackReason.isEmpty());

            ReceiverService.AudioSession retry = new ReceiverService.AudioSession(87,
                    new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
            retry.requestedOutputEngine = "oboeExperimental";
            PlaybackOutput reopened = service.openOboeOutputIfRequested(retry, 48_000, 2, true, 240);
            assertNotNull(reopened);
            assertEquals(2, nativeCalls.openCalls);
            reopened.close();
        } finally {
            OboeNativeBridge.setTestCalls(null);
            ReceiverService.currentAudio = null;
            service.onDestroy();
        }
    }

    @Test
    public void failedOpenCleanupRetainsHandleAndStartFailureSchedulesOnlyOneTrackFallback() throws Exception {
        FakeOboeCalls nativeCalls = new FakeOboeCalls();
        OboeNativeBridge.setTestCalls(nativeCalls);
        ReceiverService service = Robolectric.buildService(ReceiverService.class).create().get();
        ReceiverService.AudioSession failedOpen = new ReceiverService.AudioSession(89,
                new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
        failedOpen.requestedOutputEngine = "oboeExperimental";
        PlaybackOutput activeOutput = null;
        AudioTrack fallbackTrack = null;
        try {
            nativeCalls.openStatus = OboeNativeBridge.ERR_OPEN_FAILED;
            nativeCalls.closeFailures = 2;
            assertNull(service.openOboeOutputIfRequested(failedOpen, 48_000, 2, true, 240));
            assertEquals(1, nativeCalls.closeCalls);
            assertFalse(failedOpen.outputFallbackReason.isEmpty());

            ReceiverService.AudioSession blocked = new ReceiverService.AudioSession(90,
                    new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
            blocked.requestedOutputEngine = "oboeExperimental";
            assertNull(service.openOboeOutputIfRequested(blocked, 48_000, 2, true, 240));
            assertEquals("unclosed native handle must block another stream", 1, nativeCalls.openCalls);

            nativeCalls.openStatus = 0;
            ReceiverService.AudioSession retry = new ReceiverService.AudioSession(91,
                    new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
            retry.requestedOutputEngine = "oboeExperimental";
            activeOutput = service.openOboeOutputIfRequested(retry, 48_000, 2, true, 240);
            assertNotNull(activeOutput);
            assertEquals(2, nativeCalls.openCalls);

            ReceiverService.noteOboeRuntimeFailure(retry, new IOException("UDP receive timed out"));
            assertEquals("ordinary network failures do not schedule an engine fallback", "",
                    ReceiverService.pendingOboeFallbackReasonForUi());

            nativeCalls.startStatus = OboeNativeBridge.ERR_STREAM_FAILED;
            try { activeOutput.play(); throw new AssertionError("native start failure was accepted"); }
            catch (OboeOutputException expected) { ReceiverService.noteOboeRuntimeFailure(retry, expected); }
            activeOutput.close();
            activeOutput = null;

            ReceiverService.AudioSession fallback = new ReceiverService.AudioSession(92,
                    new ReceiverService.AudioRequest(null, null, "lowlatency"), "test-owner", 1);
            fallback.requestedOutputEngine = "oboeExperimental";
            assertNull(service.openOboeOutputIfRequested(fallback, 48_000, 2, true, 240));
            assertEquals("oboeExperimental", fallback.requestedOutputEngine);
            assertTrue(fallback.outputFallbackReason.contains("Oboe start failed"));
            ReceiverService.OpenedV2Track audioTrack = ReceiverService.openV2AudioTrack(
                    48_000, AudioFormat.CHANNEL_OUT_STEREO);
            fallbackTrack = audioTrack.track;
            ReceiverService.recordOutputOpened(fallback,
                    new AudioTrackPlaybackOutput(audioTrack.track, audioTrack.formatCode, audioTrack.frameBytes));
            assertEquals("audioTrack", fallback.actualOutputEngine);
            assertEquals(48_000, audioTrack.track.getSampleRate());
            assertTrue(fallback.outputFallbackReason.contains("Oboe start failed"));
        } finally {
            if (activeOutput != null) activeOutput.close();
            ReceiverService.releaseTrack(fallbackTrack);
            OboeNativeBridge.setTestCalls(null);
            ReceiverService.currentAudio = null;
            service.onDestroy();
        }
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

    private static void resetWaveformForTest() throws Exception {
        Field field = ReceiverService.class.getDeclaredField("WAVEFORM");
        field.setAccessible(true);
        ((PcmWaveformTap) field.get(null)).reset();
    }

    private static ReceiverService activeService() throws Exception {
        Field field = ReceiverService.class.getDeclaredField("activeService");
        field.setAccessible(true);
        return (ReceiverService) field.get(null);
    }
}
