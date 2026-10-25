package app.anpod.receiver;

import android.os.Build;
import android.net.LocalSocket;
import android.os.ParcelFileDescriptor;

import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.channels.Channels;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.net.Socket;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.locks.LockSupport;

/** Owns one authenticated record byte stream; cancel is nonblocking and awaitClosed is background-only. */
final class AoaAccessoryIo {
    static final int TRANSFER_BYTES = 16 * 1024;
    private static final long RECORD_TIMEOUT_SECONDS = 3;

    private final ReadableByteChannel input;
    private final WritableByteChannel output;
    private final AutoCloseable owner;
    private final long recordTimeoutMillis;
    private final ScheduledExecutorService cleanup = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "AnPod-AOA-IoCleanup");
        thread.setDaemon(true);
        return thread;
    });
    private final Object state = new Object();
    private final Object readLock = new Object();
    private final Object writeLock = new Object();
    private final CountDownLatch closed = new CountDownLatch(1);
    private final ChannelInput bufferedInput;
    private boolean cancelled;
    private int inFlight;
    private IOException closeFailure;

    AoaAccessoryIo(ReadableByteChannel input, WritableByteChannel output, AutoCloseable owner, long recordTimeoutMillis) {
        this.input = input;
        this.output = output;
        this.owner = owner;
        this.recordTimeoutMillis = recordTimeoutMillis;
        this.bufferedInput = new ChannelInput(input);
    }

    static AoaAccessoryIo open(ParcelFileDescriptor descriptor) throws IOException {
        if (Build.VERSION.SDK_INT < 24) throw new IOException("AOA transport requires Android API 24 or newer");
        if (descriptor == null) throw new NullPointerException("descriptor");
        FileInputStream in = null;
        FileOutputStream out = null;
        try {
            // Android 7+ FileInputStream(FileDescriptor) is non-owning; FileChannel.close interrupts its I/O.
            in = new FileInputStream(descriptor.getFileDescriptor());
            out = new FileOutputStream(descriptor.getFileDescriptor());
            return new AoaAccessoryIo(in.getChannel(), out.getChannel(), descriptor);
        } catch (RuntimeException e) {
            if (in != null) try { in.getChannel().close(); } catch (IOException close) { e.addSuppressed(close); }
            if (out != null) try { out.getChannel().close(); } catch (IOException close) { e.addSuppressed(close); }
            try { descriptor.close(); } catch (IOException close) { e.addSuppressed(close); }
            throw e;
        }
    }

    static AoaAccessoryIo open(LocalSocket socket) throws IOException {
        if (socket == null) throw new NullPointerException("socket");
        try {
            return new AoaAccessoryIo(Channels.newChannel(socket.getInputStream()),
                    Channels.newChannel(socket.getOutputStream()), socket);
        } catch (IOException | RuntimeException error) {
            try { socket.close(); } catch (IOException closeError) { error.addSuppressed(closeError); }
            if (error instanceof IOException) throw (IOException) error;
            throw new IOException("ADB LocalSocket stream open failed", error);
        }
    }

    static AoaAccessoryIo open(Socket socket) throws IOException {
        if (socket == null) throw new NullPointerException("socket");
        try {
            return new AoaAccessoryIo(Channels.newChannel(socket.getInputStream()),
                    Channels.newChannel(socket.getOutputStream()), socket);
        } catch (IOException | RuntimeException error) {
            try { socket.close(); } catch (IOException closeError) { error.addSuppressed(closeError); }
            if (error instanceof IOException) throw (IOException) error;
            throw new IOException("TCP record stream open failed", error);
        }
    }

    /** Package-private seam: tests exercise the same owner, framing, deadline, and shutdown logic. */
    AoaAccessoryIo(ReadableByteChannel input, WritableByteChannel output, AutoCloseable owner) {
        this(input, output, owner, TimeUnit.SECONDS.toMillis(RECORD_TIMEOUT_SECONDS));
    }

    AnpuRecordCodec.Record readRecord() throws IOException {
        synchronized (readLock) {
            begin();
            RecordDeadline deadline = new RecordDeadline();
            RecordInput stream = new RecordInput(bufferedInput, deadline);
            try {
                AnpuRecordCodec.Record record = AnpuRecordCodec.read(stream);
                deadline.finish();
                ensureNotCancelled();
                return record;
            } catch (IOException e) {
                cancel();
                throw e;
            } finally {
                deadline.stop();
                end();
            }
        }
    }

    void writeRecord(int type, byte[] body) throws IOException {
        synchronized (writeLock) {
            byte[] bytes = AnpuRecordCodec.encode(type, body);
            begin();
            RecordDeadline deadline = new RecordDeadline();
            deadline.start();
            try {
                ByteBuffer source = ByteBuffer.wrap(bytes);
                while (source.hasRemaining()) {
                    int count = output.write(source);
                    if (count < 0) throw new IOException("record stream write failed");
                    if (count == 0) pauseOrFail("record stream write interrupted without progress");
                }
                deadline.finish();
                ensureNotCancelled();
            } catch (IOException e) {
                cancel();
                throw e;
            } finally {
                deadline.stop();
                end();
            }
        }
    }

    /** Invalidates operations immediately; descriptor cleanup runs off-thread and waits for active I/O. */
    void cancel() {
        synchronized (state) {
            if (cancelled) return;
            cancelled = true;
        }
        cleanup.execute(this::finishClose);
    }

    /** Waits for off-thread cleanup; never call from the main thread. */
    void awaitClosed() throws IOException, InterruptedException {
        closed.await();
        synchronized (state) {
            if (closeFailure != null) throw closeFailure;
        }
    }

    private void begin() throws IOException {
        synchronized (state) {
            if (cancelled) throw new IOException("record stream is closed");
            inFlight++;
        }
    }

    private void end() {
        synchronized (state) {
            inFlight--;
            state.notifyAll();
        }
    }

    private void ensureNotCancelled() throws IOException {
        synchronized (state) {
            if (cancelled) throw new IOException("record stream was cancelled during I/O");
        }
    }

    private void finishClose() {
        IOException failure = null;
        try { input.close(); } catch (IOException e) { failure = e; }
        try { output.close(); } catch (IOException e) {
            if (failure == null) failure = e; else failure.addSuppressed(e);
        }
        synchronized (state) {
            boolean interrupted = false;
            while (inFlight != 0) {
                try { state.wait(); } catch (InterruptedException e) { interrupted = true; }
            }
            if (interrupted) Thread.currentThread().interrupt();
        }
        try { owner.close(); } catch (Exception e) {
            IOException wrapped = e instanceof IOException ? (IOException) e : new IOException("AOA descriptor close failed", e);
            if (failure == null) failure = wrapped; else failure.addSuppressed(wrapped);
        }
        synchronized (state) { closeFailure = failure; }
        closed.countDown();
        cleanup.shutdown();
    }

    private final class RecordInput extends InputStream {
        private final ChannelInput channel;
        private final RecordDeadline deadline;
        RecordInput(ChannelInput channel, RecordDeadline deadline) { this.channel = channel; this.deadline = deadline; }

        @Override public int read() throws IOException {
            byte[] one = new byte[1];
            int count = read(one, 0, 1);
            return count < 0 ? -1 : one[0] & 0xff;
        }

        @Override public int read(byte[] target, int targetOffset, int count) throws IOException {
            int read = channel.read(target, targetOffset, count);
            if (read > 0) deadline.start();
            return read;
        }
    }

    private static final class ChannelInput {
        private final ReadableByteChannel channel;
        private final byte[] transfer = new byte[TRANSFER_BYTES];
        private int offset;
        private int length;

        ChannelInput(ReadableByteChannel channel) { this.channel = channel; }

        int read(byte[] target, int targetOffset, int count) throws IOException {
            if (target == null) throw new NullPointerException("target");
            if (targetOffset < 0 || count < 0 || count > target.length - targetOffset) throw new IndexOutOfBoundsException();
            if (count == 0) return 0;
            if (offset == length) {
                int read;
                do {
                    ByteBuffer buffer = ByteBuffer.wrap(transfer);
                    read = channel.read(buffer);
                    if (read == 0) pauseOrFail("record stream read interrupted without data");
                } while (read == 0);
                if (read < 0) return -1;
                offset = 0;
                length = read;
            }
            int copied = Math.min(count, length - offset);
            System.arraycopy(transfer, offset, target, targetOffset, copied);
            offset += copied;
            return copied;
        }
    }

    private static void pauseOrFail(String message) throws IOException {
        if (Thread.currentThread().isInterrupted()) throw new IOException(message);
        LockSupport.parkNanos(TimeUnit.MILLISECONDS.toNanos(1));
    }

    private final class RecordDeadline {
        private final AtomicBoolean active = new AtomicBoolean(true);
        private ScheduledFuture<?> timeout;
        private boolean started;

        synchronized void start() {
            if (started) return;
            started = true;
            timeout = cleanup.schedule(() -> {
                if (active.compareAndSet(true, false)) AoaAccessoryIo.this.cancel();
            }, recordTimeoutMillis, TimeUnit.MILLISECONDS);
        }

        void finish() throws IOException {
            if (!active.compareAndSet(true, false)) throw new IOException("record I/O deadline expired");
            stop();
        }

        synchronized void stop() {
            if (timeout != null) timeout.cancel(false);
        }
    }
}
