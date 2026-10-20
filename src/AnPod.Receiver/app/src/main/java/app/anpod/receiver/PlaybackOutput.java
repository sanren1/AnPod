package app.anpod.receiver;

import android.media.AudioTrack;
import java.io.IOException;

/** One session's output endpoint. All writes are called by its audio worker. */
interface PlaybackOutput extends AutoCloseable {
    String engine();
    AudioTrack audioTrack();
    int sampleRate();
    int channels();
    int formatCode();
    int capacityFrames();
    int effectiveBufferFrames();
    int startThresholdFrames();
    int framesPerBurst();
    int ringCapacityFrames();
    int underrunCount() throws IOException;
    int discontinuityCount() throws IOException;
    int oboeXrunCount() throws IOException;
    /** Unsigned 32-bit API/software frame position, or -1 when unavailable. */
    long playbackHeadPosition() throws IOException;
    long pendingFrames(long acceptedFrames, long playedFrames) throws IOException;
    int writePcm16(byte[] data, int offsetBytes, int lengthBytes) throws IOException;
    int writeFloatFrames(float[] samples, int offsetFrames, int frameCount, byte[] packedScratch)
            throws IOException;
    void play() throws IOException;
    void requestPause() throws IOException;
    void pauseAndFlush() throws IOException;
    void setVolume(float volume) throws IOException;
    int[] info();
    int[] metrics() throws IOException;
    long[] timestamp() throws IOException;
    @Override void close() throws IOException;
}
