package app.anpod.receiver;

import android.media.AudioFormat;
import android.media.AudioTrack;
import com.anpod.server.Pcm24PackedCodec;
import java.io.IOException;

/** Extracted from the AudioShare-adapted Receiver path; see upstream/PATCHES.md B14. */
final class AudioTrackPlaybackOutput implements PlaybackOutput {
    private final AudioTrack track;
    private final int formatCode;
    private final int channels;
    private final int sampleRate;
    private final int frameBytes;

    AudioTrackPlaybackOutput(AudioTrack track, int formatCode, int frameBytes) {
        if (track == null) throw new IllegalArgumentException("track");
        this.track = track;
        this.formatCode = formatCode;
        this.channels = track.getChannelCount();
        this.sampleRate = track.getSampleRate();
        this.frameBytes = frameBytes;
    }

    @Override public String engine() { return "audioTrack"; }
    @Override public AudioTrack audioTrack() { return track; }
    @Override public int sampleRate() { return sampleRate; }
    @Override public int channels() { return channels; }
    @Override public int formatCode() { return formatCode; }
    @Override public int capacityFrames() {
        return android.os.Build.VERSION.SDK_INT >= 24 ? track.getBufferCapacityInFrames()
                : track.getBufferSizeInFrames();
    }
    @Override public int effectiveBufferFrames() { return track.getBufferSizeInFrames(); }
    @Override public int startThresholdFrames() {
        return android.os.Build.VERSION.SDK_INT >= 31 ? track.getStartThresholdInFrames() : -1;
    }
    @Override public int framesPerBurst() { return 0; }
    @Override public int ringCapacityFrames() { return 0; }
    @Override public int underrunCount() {
        return android.os.Build.VERSION.SDK_INT >= 24 ? track.getUnderrunCount() : -1;
    }
    @Override public int discontinuityCount() { return underrunCount(); }
    @Override public int oboeXrunCount() { return -1; }
    @Override public long playbackHeadPosition() throws IOException {
        try { return Integer.toUnsignedLong(track.getPlaybackHeadPosition()); }
        catch (IllegalStateException error) { throw new IOException("AudioTrack playback position unavailable", error); }
    }
    @Override public long pendingFrames(long acceptedFrames, long playedFrames) {
        return com.anpod.server.Pcm16FrameStream.pendingFrames(acceptedFrames, playedFrames);
    }

    @Override public int writePcm16(byte[] data, int offsetBytes, int lengthBytes) {
        return track.write(data, offsetBytes, lengthBytes, AudioTrack.WRITE_BLOCKING);
    }

    @Override public int writeFloatFrames(float[] samples, int offsetFrames, int frameCount,
            byte[] packedScratch) throws IOException {
        if (formatCode == 2) {
            Pcm24PackedCodec.encodeFromFloat(samples, offsetFrames * channels, frameCount * channels,
                    packedScratch, 0);
            int written = track.write(packedScratch, 0, frameCount * frameBytes, AudioTrack.WRITE_BLOCKING);
            if (written > 0 && written % frameBytes != 0)
                throw new IOException("AudioTrack returned a partial packed PCM24 frame");
            return written <= 0 ? written : written / frameBytes;
        }
        int written = track.write(samples, offsetFrames * channels, frameCount * channels, AudioTrack.WRITE_BLOCKING);
        if (written > 0 && written % channels != 0)
            throw new IOException("AudioTrack returned a partial Float32 frame");
        return written <= 0 ? written : written / channels;
    }

    @Override public void play() throws IOException {
        try { track.play(); }
        catch (IllegalStateException error) { throw new IOException("AudioTrack start failed", error); }
    }
    @Override public void requestPause() { }
    @Override public void pauseAndFlush() throws IOException {
        String failure = "";
        try { track.pause(); }
        catch (RuntimeException error) { failure = ReceiverService.appendFailure(failure, ReceiverService.describeFailure(error)); }
        try { track.flush(); }
        catch (RuntimeException error) { failure = ReceiverService.appendFailure(failure, ReceiverService.describeFailure(error)); }
        if (!failure.isEmpty()) throw new IOException("AudioTrack pause/flush failed: " + failure);
    }
    @Override public void setVolume(float volume) throws IOException {
        try { track.setVolume(volume); }
        catch (IllegalStateException error) { throw new IOException("AudioTrack volume change failed", error); }
    }
    @Override public int[] info() { return null; }
    @Override public int[] metrics() { return null; }
    @Override public long[] timestamp() { return null; }
    @Override public void close() throws IOException {
        String failure = ReceiverService.releaseTrack(track);
        if (!failure.isEmpty()) throw new IOException("AudioTrack close failed: " + failure);
    }
}
