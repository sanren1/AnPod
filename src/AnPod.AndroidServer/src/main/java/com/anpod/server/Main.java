package com.anpod.server;

import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTrack;
import android.net.LocalSocket;
import android.net.LocalSocketAddress;
import android.os.Build;
import android.os.Process;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;

/**
 * Headless Android audio receiver.
 *
 * <p>Derived from ysbing/AudioShare server Main.java (LGPL-3.0-or-later). The
 * wire protocol is unchanged: connect the abstract LocalSocket, send the
 * connect code, read the 8-byte little-endian format header, then stream raw
 * PCM16 into an AudioTrack.
 *
 * <p>Changes from upstream (see upstream/PATCHES.md):
 * <ul>
 *   <li>every header field is validated, not just the sample rate;</li>
 *   <li>lifecycle events are written to stderr in a machine-greppable form so
 *       the Windows side can mirror them into its own log;</li>
 *   <li>AudioTrack failures always stop and release the track.</li>
 * </ul>
 *
 * <p>This is not an APK and is never installed. It is pushed to
 * {@code /data/local/tmp} and started with {@code app_process}.
 */
public final class Main {

    private static final int SOCKET_AUDIO_BUFFER_BYTES = 32 * 1024;

    /* Accepted format range. Mirrors the validation in ConnectionManager. */
    private static final int MIN_SAMPLE_RATE = 8000;
    private static final int MAX_SAMPLE_RATE = 192000;
    private static final int REQUIRED_BITS_PER_SAMPLE = 16;

    private static final int HEADER_BYTES = 8;

    private Main() {
    }

    public static void main(String[] args) {
        try {
            Options options = Options.parse(args);
            log("START socket=" + options.socketName);

            try (LocalSocket socket = connect(options.socketName, options.connectCode)) {
                InputStream input = socket.getInputStream();

                byte[] header = readFully(input, HEADER_BYTES);
                int sampleRate = readInt32LE(header, 0);
                int channels = readInt16LE(header, 4);
                int bitsPerSample = readInt16LE(header, 6);

                validateFormat(sampleRate, channels, bitsPerSample);

                int channelMask = (channels == 1)
                        ? AudioFormat.CHANNEL_OUT_MONO
                        : AudioFormat.CHANNEL_OUT_STEREO;

                log("FORMAT sampleRate=" + sampleRate + " channels=" + channels + " bits=" + bitsPerSample);

                play(input, sampleRate, channelMask, options.playbackMode);
            }

            log("STOP");
        } catch (Throwable throwable) {
            log("ERROR " + throwable.getClass().getSimpleName() + ": " + throwable.getMessage());
            throwable.printStackTrace();
            System.exit(1);
        }
    }

    /** Rejects anything the Windows side should never have sent (plan §3.3). */
    private static void validateFormat(int sampleRate, int channels, int bitsPerSample)
            throws IOException {
        if (sampleRate < MIN_SAMPLE_RATE || sampleRate > MAX_SAMPLE_RATE) {
            throw new IOException("UNSUPPORTED_SAMPLE_RATE " + sampleRate);
        }

        if (channels != 1 && channels != 2) {
            throw new IOException("UNSUPPORTED_CHANNELS " + channels);
        }

        if (bitsPerSample != REQUIRED_BITS_PER_SAMPLE) {
            throw new IOException("UNSUPPORTED_BITS_PER_SAMPLE " + bitsPerSample);
        }
    }

    private static LocalSocket connect(String abstractName, String connectCode) throws IOException {
        LocalSocket localSocket = new LocalSocket();
        localSocket.connect(new LocalSocketAddress(abstractName));

        try {
            localSocket.setReceiveBufferSize(SOCKET_AUDIO_BUFFER_BYTES);
        } catch (IOException ignored) {
            // Buffer tuning is optional and must never prevent audio playback.
        }

        OutputStream output = localSocket.getOutputStream();
        output.write(connectCode.getBytes("UTF-8"));
        output.flush();
        return localSocket;
    }

    private static byte[] readFully(InputStream input, int length) throws IOException {
        byte[] buffer = new byte[length];
        int offset = 0;

        while (offset < length) {
            int read = input.read(buffer, offset, length - offset);
            if (read == -1) {
                throw new IOException("FORMAT_HEADER_INCOMPLETE");
            }
            offset += read;
        }

        return buffer;
    }

    private static int readInt32LE(byte[] buffer, int offset) {
        return (buffer[offset] & 0xFF)
                | ((buffer[offset + 1] & 0xFF) << 8)
                | ((buffer[offset + 2] & 0xFF) << 16)
                | ((buffer[offset + 3] & 0xFF) << 24);
    }

    private static int readInt16LE(byte[] buffer, int offset) {
        return (buffer[offset] & 0xFF) | ((buffer[offset + 1] & 0xFF) << 8);
    }

    private static void play(InputStream input, int sampleRate, int channelMask, String requestedMode) throws IOException {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO);

        int minBuffer = AudioTrack.getMinBufferSize(
                sampleRate, channelMask, AudioFormat.ENCODING_PCM_16BIT);

        if (minBuffer <= 0) {
            throw new IOException("AUDIO_TRACK_UNSUPPORTED_FORMAT");
        }

        // Keep the current track capacity in balanced mode; low latency reduces
        // only the effective application buffer after the track is built.
        int trackBufferBytes = minBuffer * 2;

        AudioTrack.Builder builder = new AudioTrack.Builder()
                .setAudioAttributes(new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build())
                .setAudioFormat(new AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(sampleRate)
                        .setChannelMask(channelMask)
                        .build())
                .setBufferSizeInBytes(trackBufferBytes)
                .setTransferMode(AudioTrack.MODE_STREAM);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY);
        }

        AudioTrack audioTrack = builder.build();
        Pcm16FrameStream pcm = new Pcm16FrameStream(channelsForMask(channelMask));
        Pcm16FrameStream.PlaybackHeadTracker playbackHead = new Pcm16FrameStream.PlaybackHeadTracker();
        long totalFramesWritten = 0;

        try {
            // A track that failed to initialise reports it here rather than by
            // throwing, and play() would then fail with a message that says
            // nothing about the format or the device.
            if (audioTrack.getState() != AudioTrack.STATE_INITIALIZED) {
                throw new IOException("AUDIO_TRACK_NOT_INITIALIZED state=" + audioTrack.getState()
                        + " sampleRate=" + sampleRate
                        + " channelMask=" + channelMask
                        + " bufferBytes=" + trackBufferBytes);
            }

            // getBufferCapacityInFrames() was added in API 24. On API 23 the
            // configured size is the best available capacity for this track.
            int capacityFrames = Build.VERSION.SDK_INT >= 24
                    ? audioTrack.getBufferCapacityInFrames()
                    : audioTrack.getBufferSizeInFrames();
            int frameBytes = channelsForMask(channelMask) * 2;
            int minBufferFrames = Math.max(1, (minBuffer + frameBytes - 1) / frameBytes);
            String actualMode = requestedMode;
            int requestedEffectiveFrames = -1;
            int acceptedEffectiveFrames = -1;
            int requestedStartFrames = -1;
            int acceptedStartFrames = -1;
            boolean lowLatencyBufferApplied = false;
            boolean startThresholdRequestLimited = false;
            if ("low-latency".equals(requestedMode)) {
                if (Build.VERSION.SDK_INT >= 24 && capacityFrames > 0) {
                    int desiredFrames = Math.max(1,
                            sampleRate * Pcm16FrameStream.LOW_LATENCY_AUDIO_TRACK_BUFFER_MS / 1000);
                    requestedEffectiveFrames = Pcm16FrameStream.requestedAudioTrackBufferFrames(
                            sampleRate, capacityFrames, true);
                    acceptedEffectiveFrames = audioTrack.setBufferSizeInFrames(requestedEffectiveFrames);
                    if (acceptedEffectiveFrames < 0) {
                        actualMode = "balanced-fallback";
                        log("BUFFER_SIZE_REQUEST_REJECTED requested=" + requestedEffectiveFrames
                                + " result=" + acceptedEffectiveFrames + "; retaining system buffer");
                    } else {
                        lowLatencyBufferApplied = true;
                        if (acceptedEffectiveFrames != requestedEffectiveFrames || capacityFrames < desiredFrames) {
                            actualMode = "low-latency-limited";
                            log("BUFFER_SIZE_LIMITED requested=" + requestedEffectiveFrames
                                    + " accepted=" + acceptedEffectiveFrames);
                        }
                    }
                    if (Build.VERSION.SDK_INT >= 31 && acceptedEffectiveFrames > 0
                            && !"balanced-fallback".equals(actualMode)) {
                        requestedStartFrames = Math.min(acceptedEffectiveFrames, Math.max(1,
                                sampleRate * Pcm16FrameStream.LOW_LATENCY_START_THRESHOLD_MS / 1000));
                        acceptedStartFrames = audioTrack.setStartThresholdInFrames(requestedStartFrames);
                        startThresholdRequestLimited = acceptedStartFrames < 0
                                || acceptedStartFrames != requestedStartFrames;
                        if (acceptedStartFrames < 0) log("START_THRESHOLD_REQUEST_REJECTED result=" + acceptedStartFrames);
                    } else if (Build.VERSION.SDK_INT >= 31 && acceptedEffectiveFrames == 0)
                        startThresholdRequestLimited = true;
                } else {
                    actualMode = "balanced-fallback";
                    log("LOW_LATENCY_UNSUPPORTED api=" + Build.VERSION.SDK_INT + "; retaining system buffer");
                }
            } else if (Build.VERSION.SDK_INT >= 24 && capacityFrames > 0) {
                int desiredFrames = Math.max(1,
                        sampleRate * Pcm16FrameStream.BALANCED_AUDIO_TRACK_BUFFER_MS / 1000);
                requestedEffectiveFrames = Pcm16FrameStream.requestedAudioTrackBufferFrames(
                        sampleRate, capacityFrames, false);
                acceptedEffectiveFrames = audioTrack.setBufferSizeInFrames(requestedEffectiveFrames);
                if (acceptedEffectiveFrames < 0) {
                    actualMode = "balanced-fallback";
                    log("BALANCED_BUFFER_REQUEST_REJECTED requested=" + requestedEffectiveFrames
                            + " result=" + acceptedEffectiveFrames + "; retaining system buffer");
                } else if (capacityFrames < desiredFrames || acceptedEffectiveFrames != requestedEffectiveFrames) {
                    actualMode = "balanced-limited";
                }
            } else {
                actualMode = "balanced-system";
            }

            int effectiveFrames = audioTrack.getBufferSizeInFrames();
            int startThresholdFrames = Build.VERSION.SDK_INT >= 31
                    ? audioTrack.getStartThresholdInFrames() : -1;
            if (lowLatencyBufferApplied && effectiveFrames != requestedEffectiveFrames) {
                actualMode = "low-latency-limited";
                log("BUFFER_SIZE_ACTUAL requested=" + requestedEffectiveFrames + " actual=" + effectiveFrames);
            }
            if (Build.VERSION.SDK_INT >= 31 && lowLatencyBufferApplied
                    && (startThresholdFrames != requestedStartFrames || startThresholdRequestLimited)) {
                actualMode = "low-latency-limited";
                log("START_THRESHOLD_LIMITED requested=" + requestedStartFrames + " actual=" + startThresholdFrames);
            }
            if ("balanced".equals(actualMode) && effectiveFrames != requestedEffectiveFrames)
                actualMode = "balanced-limited";
            int preRollFrames = lowLatencyBufferApplied
                    ? (startThresholdFrames > 0
                            ? Math.min(effectiveFrames, startThresholdFrames) : effectiveFrames)
                    : Math.min(effectiveFrames, minBufferFrames);
            log("AUDIO requestedMode=" + requestedMode
                    + " actualMode=" + actualMode
                    + " actualSampleRate=" + audioTrack.getSampleRate()
                    + " actualChannels=" + audioTrack.getChannelCount()
                    + " minBufferBytes=" + minBuffer
                    + " capacityFrames=" + capacityFrames
                    + " effectiveFrames=" + effectiveFrames
                    + " requestedBufferFrames=" + requestedEffectiveFrames
                    + " acceptedBufferFrames=" + acceptedEffectiveFrames
                    + " requestedStartThresholdFrames=" + requestedStartFrames
                    + " acceptedStartThresholdFrames=" + acceptedStartFrames
                    + " preRollFrames=" + preRollFrames
                    + " startThresholdFrames=" + startThresholdFrames
                    + " performance=" + getActualPerformanceMode(audioTrack));

            playbackHead.reset(audioTrack.getPlaybackHeadPosition());

            attachPlayerContext(audioTrack);

            byte[] readBuffer = new byte[Math.max(frameBytes, minBuffer + frameBytes - 1)];
            int backlogLimitFrames = Pcm16FrameStream.backlogLimitFrames(
                    audioTrack.getSampleRate(), preRollFrames, lowLatencyBufferApplied);
            pcm.setBacklogLimit(backlogLimitFrames,
                    (audioTrack.getSampleRate() * 3 + 999) / 1000);
            log("PCM_BACKLOG limitFrames=" + backlogLimitFrames);
            int preRollTargetBytes = preRollFrames * frameBytes;
            int preRolledBytes = 0;
            while (preRolledBytes < preRollTargetBytes) {
                int read = pcm.readFrames(input, readBuffer,
                        Math.min(readBuffer.length, preRollTargetBytes - preRolledBytes));
                if (read < 0) break;
                writeFrames(pcm, audioTrack, readBuffer, read);
                preRolledBytes += read;
                totalFramesWritten += read / frameBytes;
            }

            audioTrack.play();
            int initialUnderruns = Build.VERSION.SDK_INT >= 24 ? audioTrack.getUnderrunCount() : -1;
            long initialPlayedFrames = playbackHead.update(audioTrack.getPlaybackHeadPosition());
            log("PLAYING preRollFrames=" + (preRolledBytes / frameBytes)
                    + " pendingFrames=" + Pcm16FrameStream.pendingFrames(totalFramesWritten, initialPlayedFrames)
                    + " underruns=" + initialUnderruns);
            long lastUnderrunLog = System.currentTimeMillis();

            while (!Thread.currentThread().isInterrupted()) {
                int bytesRead = pcm.readFrames(input, readBuffer, readBuffer.length);
                if (bytesRead < 0) {
                    break;
                }
                writeFrames(pcm, audioTrack, readBuffer, bytesRead);
                totalFramesWritten += bytesRead / frameBytes;
                long now = System.currentTimeMillis();
                if (now - lastUnderrunLog >= 5000) {
                    try {
                        long playedFrames = playbackHead.update(audioTrack.getPlaybackHeadPosition());
                        int underruns = Build.VERSION.SDK_INT >= 24 ? audioTrack.getUnderrunCount() : -1;
                        log("PLAYBACK underruns=" + underruns
                                + " pendingFrames=" + Pcm16FrameStream.pendingFrames(totalFramesWritten, playedFrames)
                                + " droppedBacklogFrames=" + pcm.getDroppedBacklogFrames());
                    } catch (RuntimeException unavailable) {
                        log("PLAYBACK diagnostics=unavailable droppedBacklogFrames=" + pcm.getDroppedBacklogFrames());
                    }
                    lastUnderrunLog = now;
                }
            }
        } finally {
            try {
                long playedFrames = playbackHead.update(audioTrack.getPlaybackHeadPosition());
                int underruns = Build.VERSION.SDK_INT >= 24 ? audioTrack.getUnderrunCount() : -1;
                log("PLAYBACK finalUnderruns=" + underruns
                        + " pendingFrames=" + Pcm16FrameStream.pendingFrames(totalFramesWritten, playedFrames)
                        + " droppedBacklogFrames=" + pcm.getDroppedBacklogFrames());
            } catch (Throwable unavailable) {
                log("PLAYBACK final=unavailable droppedBacklogFrames=" + pcm.getDroppedBacklogFrames());
            }
            // Teardown must never replace the real failure. Under app_process
            // there is no ActivityThread application, so AudioTrack.stop() can
            // throw NullPointerException out of PlayerBase (it dereferences a
            // null Context). Catching only IllegalStateException let that NPE
            // escape the finally block and overwrite the original cause, which
            // made every playback failure look identical and undiagnosable.
            try {
                audioTrack.stop();
            } catch (Throwable stopFailure) {
                log("STOP_FAILED " + stopFailure.getClass().getSimpleName());
            }

            try {
                audioTrack.release();
            } catch (Throwable releaseFailure) {
                log("RELEASE_FAILED " + releaseFailure.getClass().getSimpleName());
            }

            log("RELEASED");
        }
    }

    /**
     * Gives the track's {@code PlayerBase} a {@link Context}, which it needs
     * before {@code play()} or {@code stop()} will work.
     *
     * <p>Why this is done the awkward way:
     * <ul>
     *   <li>{@code AudioTrack.Builder.setContext()} cannot be used. On the ROM
     *       this was developed against (vivo, Android 16) passing a context
     *       there makes {@code native_setup} fail outright with
     *       {@code UnsupportedOperationException: Cannot create AudioTrack}.
     *       Building first and injecting afterwards works, because by then the
     *       native track already exists.</li>
     *   <li>{@code ActivityThread.systemMain()} cannot be used either — it
     *       attaches this process as the system process. A bare
     *       {@code ActivityThread} instance is enough for
     *       {@code getSystemContext()}, and has no such side effect.</li>
     *   <li>{@code ActivityThread}'s constructor creates a {@code Handler}, so
     *       the calling thread needs a {@code Looper}.</li>
     * </ul>
     *
     * <p>Without this, {@code PlayerBase.updateState()} dereferences the null
     * context on the way into {@code play()}, so no audio is ever produced.
     * ROMs that null-guard the context do not need any of this; the injection is
     * best-effort and a failure leaves the previous behaviour untouched.
     */
    private static void attachPlayerContext(AudioTrack audioTrack) {
        Context context = resolveSystemContext();
        if (context == null) {
            log("CONTEXT_UNAVAILABLE playback may fail on this ROM");
            return;
        }

        for (Class<?> type = audioTrack.getClass(); type != null; type = type.getSuperclass()) {
            for (Field field : type.getDeclaredFields()) {
                if (field.getType() != Context.class) {
                    continue;
                }

                try {
                    field.setAccessible(true);
                    field.set(audioTrack, context);
                    log("CONTEXT_ATTACHED " + type.getSimpleName() + "." + field.getName());
                    return;
                } catch (Throwable throwable) {
                    log("CONTEXT_ATTACH_FAILED " + type.getSimpleName() + "." + field.getName()
                            + " " + throwable.getClass().getSimpleName());
                    return;
                }
            }
        }

        log("CONTEXT_FIELD_NOT_FOUND");
    }

    /** A system {@code Context}, or null when the platform will not give us one. */
    private static Context resolveSystemContext() {
        try {
            android.os.Looper.prepare();
        } catch (Throwable ignored) {
            // Already prepared on this thread; that is fine.
        }

        try {
            Class<?> activityThreadClass = Class.forName("android.app.ActivityThread");
            Constructor<?> constructor = activityThreadClass.getDeclaredConstructor();
            constructor.setAccessible(true);
            Object activityThread = constructor.newInstance();

            Method getSystemContext = activityThreadClass.getMethod("getSystemContext");
            return (Context) getSystemContext.invoke(activityThread);
        } catch (Throwable throwable) {
            log("CONTEXT_RESOLVE_FAILED " + throwable.getClass().getSimpleName());
            return null;
        }
    }

    private static int channelsForMask(int channelMask) {
        return channelMask == AudioFormat.CHANNEL_OUT_MONO ? 1 : 2;
    }

    private static void writeFrames(Pcm16FrameStream pcm, final AudioTrack track, byte[] buffer, int length)
            throws IOException {
        pcm.writeFramesForChannels((data, offset, count) -> track.write(data, offset, count), buffer, length);
    }

    private static String getActualPerformanceMode(AudioTrack track) {
        if (Build.VERSION.SDK_INT < 26) return "unsupported";
        try {
            Method method = AudioTrack.class.getMethod("getPerformanceMode");
            int mode = ((Integer) method.invoke(track)).intValue();
            if (mode == AudioTrack.PERFORMANCE_MODE_LOW_LATENCY) return "low-latency";
            if (mode == AudioTrack.PERFORMANCE_MODE_NONE) return "none";
            return "" + mode;
        } catch (Throwable unavailable) {
            return "unavailable";
        }
    }

    private static void log(String message) {
        // Mirrored into the Windows log by AndroidProcessSession.
        System.err.println("ANPOD " + message);
    }
}
