package app.anpod.receiver;

import static org.junit.Assert.*;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.AsynchronousCloseException;
import java.nio.channels.ReadableByteChannel;
import java.nio.channels.WritableByteChannel;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.Test;

public class AoaAccessoryIoTest {
    @Test public void coalescedRecordsAreRetainedAndWritesAdvanceShortChunks() throws Exception {
        byte[] record = AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, handshakeBody());
        FakeInput input = new FakeInput(concat(record, record), Integer.MAX_VALUE, false);
        FakeOutput output = new FakeOutput(Integer.MAX_VALUE, -1);
        Owner owner = new Owner(input, output);
        AoaAccessoryIo io = new AoaAccessoryIo(input, output, owner);
        assertArrayEquals(handshakeBody(), io.readRecord().body);
        assertArrayEquals(handshakeBody(), io.readRecord().body);
        output.maxWrite = 3;
        io.writeRecord(AnpuRecordCodec.HANDSHAKE, handshakeBody());
        assertArrayEquals(record, output.bytes.toByteArray());
        io.cancel();
        io.awaitClosed();
        assertEquals(1, owner.closes.get());
    }

    @Test public void cancelUnblocksIdleReadAndClosesOwnerAfterIoExits() throws Exception {
        FakeInput input = new FakeInput(new byte[0], 1, true);
        FakeOutput output = new FakeOutput(Integer.MAX_VALUE, -1);
        Owner owner = new Owner(input, output);
        AoaAccessoryIo io = new AoaAccessoryIo(input, output, owner);
        AtomicInteger failures = new AtomicInteger();
        Thread reader = new Thread(() -> {
            try { io.readRecord(); } catch (IOException expected) { failures.incrementAndGet(); }
        });
        reader.start();
        assertTrue(input.readEntered.await(2, TimeUnit.SECONDS));
        io.cancel();
        reader.join(2000);
        assertFalse(reader.isAlive());
        assertTrue(owner.done.await(2, TimeUnit.SECONDS));
        assertEquals(1, failures.get());
        assertEquals(1, owner.closes.get());
        try { io.writeRecord(AnpuRecordCodec.HANDSHAKE, handshakeBody()); fail("closed write accepted"); }
        catch (IOException expected) { }
    }

    @Test public void zeroLengthReadsRemainCancelableBeforeFirstByte() throws Exception {
        byte[] record = AnpuRecordCodec.encode(AnpuRecordCodec.HANDSHAKE, handshakeBody());
        FakeInput input = new FakeInput(record, 0, false);
        FakeOutput output = new FakeOutput(Integer.MAX_VALUE, -1);
        Owner owner = new Owner(input, output);
        AoaAccessoryIo io = new AoaAccessoryIo(input, output, owner);
        AtomicInteger failures = new AtomicInteger();
        Thread reader = new Thread(() -> {
            try { io.readRecord(); } catch (IOException expected) { failures.incrementAndGet(); }
        });
        reader.start();
        assertTrue(input.readEntered.await(2, TimeUnit.SECONDS));
        io.cancel();
        reader.join(2000);
        assertFalse(reader.isAlive());
        assertEquals(1, failures.get());
        assertTrue(owner.done.await(2, TimeUnit.SECONDS));
        assertEquals(1, owner.closes.get());
    }

    @Test public void partialRecordDeadlineClosesChannelAndOwner() throws Exception {
        FakeInput input = new FakeInput(new byte[] {'A'}, 1, true);
        FakeOutput output = new FakeOutput(Integer.MAX_VALUE, -1);
        Owner owner = new Owner(input, output);
        AoaAccessoryIo io = new AoaAccessoryIo(input, output, owner, 40);
        AtomicInteger failures = new AtomicInteger();
        Thread reader = new Thread(() -> {
            try { io.readRecord(); } catch (IOException expected) { failures.incrementAndGet(); }
        });
        reader.start();
        reader.join(2500);
        assertFalse(reader.isAlive());
        assertEquals(1, failures.get());
        assertTrue(owner.done.await(2, TimeUnit.SECONDS));
        assertEquals(1, owner.closes.get());
    }

    @Test public void partialWriteFailureIsFatalAndOwnerClosesOnce() throws Exception {
        FakeInput input = new FakeInput(new byte[0], 1, false);
        FakeOutput output = new FakeOutput(2, 3);
        Owner owner = new Owner(input, output);
        AoaAccessoryIo io = new AoaAccessoryIo(input, output, owner);
        try { io.writeRecord(AnpuRecordCodec.HANDSHAKE, handshakeBody()); fail("partial write succeeded"); }
        catch (IOException expected) { assertTrue(expected.getMessage().contains("injected")); }
        assertTrue(owner.done.await(2, TimeUnit.SECONDS));
        assertEquals(3, output.bytes.size());
        assertEquals(1, owner.closes.get());
    }

    private static byte[] handshakeBody() {
        byte[] body = new byte[96];
        body[0] = 'A'; body[1] = 'N'; body[2] = 'P'; body[3] = '5';
        body[4] = 2; body[5] = 1; body[6] = 0; body[7] = 96;
        return body;
    }

    private static byte[] concat(byte[] a, byte[] b) {
        byte[] out = Arrays.copyOf(a, a.length + b.length);
        System.arraycopy(b, 0, out, a.length, b.length);
        return out;
    }

    private static final class FakeInput implements ReadableByteChannel {
        final byte[] bytes;
        final int maxRead;
        final boolean blockAtEnd;
        final CountDownLatch readEntered = new CountDownLatch(1);
        private int offset;
        private volatile boolean open = true;

        FakeInput(byte[] bytes, int maxRead, boolean blockAtEnd) {
            this.bytes = bytes; this.maxRead = maxRead; this.blockAtEnd = blockAtEnd;
        }

        @Override public int read(ByteBuffer dst) throws IOException {
            readEntered.countDown();
            synchronized (this) {
                if (!open) throw new AsynchronousCloseException();
                if (offset == bytes.length) {
                    if (!blockAtEnd) return -1;
                    while (open) {
                        try { wait(); } catch (InterruptedException e) { Thread.currentThread().interrupt(); throw new IOException(e); }
                    }
                    throw new AsynchronousCloseException();
                }
                int count = Math.min(Math.min(dst.remaining(), maxRead), bytes.length - offset);
                dst.put(bytes, offset, count);
                offset += count;
                return count;
            }
        }

        @Override public boolean isOpen() { return open; }
        @Override public synchronized void close() { open = false; notifyAll(); }
    }

    private static final class FakeOutput implements WritableByteChannel {
        final ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        volatile int maxWrite;
        final int failAfter;
        private volatile boolean open = true;
        FakeOutput(int maxWrite, int failAfter) { this.maxWrite = maxWrite; this.failAfter = failAfter; }
        @Override public synchronized int write(ByteBuffer src) throws IOException {
            if (!open) throw new AsynchronousCloseException();
            if (failAfter >= 0 && bytes.size() >= failAfter) throw new IOException("injected partial write failure");
            int count = Math.min(Math.min(src.remaining(), maxWrite), failAfter < 0 ? Integer.MAX_VALUE : failAfter - bytes.size());
            for (int i = 0; i < count; i++) bytes.write(src.get());
            return count;
        }
        @Override public boolean isOpen() { return open; }
        @Override public void close() { open = false; }
    }

    private static final class Owner implements AutoCloseable {
        final FakeInput input;
        final FakeOutput output;
        final CountDownLatch done = new CountDownLatch(1);
        final AtomicInteger closes = new AtomicInteger();
        Owner(FakeInput input, FakeOutput output) { this.input = input; this.output = output; }
        @Override public void close() {
            assertFalse("owner closed before channels", input.open || output.open);
            closes.incrementAndGet();
            done.countDown();
        }
    }
}
