package app.anpod.receiver;

import android.app.ActivityManager;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructPollfd;
import android.content.pm.ServiceInfo;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import java.io.FileNotFoundException;
import java.io.FileDescriptor;
import java.io.InterruptedIOException;
import java.io.IOException;
import java.net.Inet4Address;
import java.net.NetworkInterface;
import java.net.SocketException;
import java.util.Enumeration;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;

public final class ReceiverStateProvider extends ContentProvider {
    private static final String BINDING_URI = "content://app.anpod.receiver.state/binding";
    private static final String INTERFACES_URI = "content://app.anpod.receiver.state/interfaces";
    private static final String NETWORK_URI = "content://app.anpod.receiver.state/network";
    private static final String[] INTERFACE_COLUMNS = {"address", "prefixLength", "interfaceIndex", "interfaceName"};
    private static final String[] NETWORK_COLUMNS = {"networkAudioVersion", "networkAudioState", "networkAudioFailure", "pcmBytesWritten", "networkAudioAddress", "networkAudioPort"};
    private static final String[] BINDING_COLUMNS = {"bindingState", "bindingCommitment", "bindingFailure"};
    private static final long BINDING_READ_TIMEOUT_MS = 15_000;
    private static final int POLL_SLICE_MS = 100;
    private static final String[] COLUMNS = {
            "applicationId", "versionName", "versionCode", "serviceActive",
            "foreground", "healthy", "audioActive", "actualMode", "actualSampleRate",
            "bufferCapacityFrames", "effectiveBufferFrames", "startThresholdFrames", "underrunCount",
            "udpJitterTargetFrames", "udpExpiredFrames", "tcpDroppedBacklogFrames",
            "startedAtElapsedMs", "error"
    };
    private final AtomicBoolean bindingWriteInProgress = new AtomicBoolean();
    private final Object bindingStateLock = new Object();
    private volatile String bindingFailure = "";
    private BindingStore bindingStore;

    @Override
    public boolean onCreate() {
        Context context = getContext();
        bindingStore = new BindingStore(context.getFilesDir());
        return true;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection,
            String[] selectionArgs, String sortOrder) {
        boolean healthUri = isPath(uri, "health");
        boolean interfacesUri = isPath(uri, "interfaces");
        boolean networkUri = isPath(uri, "network");
        if (!BINDING_URI.equals(uri.toString()) && !healthUri && !interfacesUri && !networkUri
                && (!"app.anpod.receiver.state".equals(uri.getAuthority())
                || uri.getPathSegments().size() != 1)) {
            throw new IllegalArgumentException("Unsupported receiver status URI");
        }

        if (BINDING_URI.equals(uri.toString())) {
            if (projection == null || !sameColumns(projection, BINDING_COLUMNS)
                    || selection != null || selectionArgs != null || sortOrder != null)
                throw new IllegalArgumentException("Unsupported receiver binding query");
            return queryBinding();
        }

        if (interfacesUri) return queryInterfaces(projection, selection, selectionArgs, sortOrder);
        if (networkUri) return queryNetwork(projection, selection, selectionArgs, sortOrder);
        if (!healthUri) throw new IllegalArgumentException("Unsupported receiver status URI");

        String[] columns = projection == null ? COLUMNS : projection;
        for (String column : columns) {
            if (!contains(COLUMNS, column)) throw new IllegalArgumentException("Unsupported receiver status column");
        }

        boolean active = false;
        boolean foreground = false;
        Context context = getContext();
        ActivityManager manager = context.getSystemService(ActivityManager.class);
        for (ActivityManager.RunningServiceInfo service : manager.getRunningServices(Integer.MAX_VALUE)) {
            if (ReceiverService.class.getName().equals(service.service.getClassName())) {
                active = service.started;
                foreground = service.foreground;
                break;
            }
        }
        boolean healthy = active && foreground && ReceiverService.foregroundReady
                && ReceiverService.startFailure.isEmpty();
        MatrixCursor result = new MatrixCursor(columns, 1);
        Object[] values = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) {
            switch (columns[i]) {
                case "applicationId": values[i] = BuildConfig.APPLICATION_ID; break;
                case "versionName": values[i] = BuildConfig.VERSION_NAME; break;
                case "versionCode": values[i] = BuildConfig.VERSION_CODE; break;
                case "serviceActive": values[i] = active; break;
                case "foreground": values[i] = foreground; break;
                case "healthy": values[i] = healthy; break;
                case "audioActive": values[i] = ReceiverService.audioActive; break;
                case "actualMode": values[i] = ReceiverService.actualMode; break;
                case "actualSampleRate": values[i] = metric(ReceiverService.actualSampleRate); break;
                case "bufferCapacityFrames": values[i] = metric(ReceiverService.bufferCapacityFrames); break;
                case "effectiveBufferFrames": values[i] = metric(ReceiverService.effectiveBufferFrames); break;
                case "startThresholdFrames": values[i] = metric(ReceiverService.startThresholdFrames); break;
                case "underrunCount": values[i] = metric(ReceiverService.underrunCount); break;
                case "udpJitterTargetFrames": values[i] = metric(ReceiverService.udpJitterTargetFrames); break;
                case "udpExpiredFrames": values[i] = ReceiverService.udpExpiredFrames; break;
                case "tcpDroppedBacklogFrames": values[i] = ReceiverService.tcpDroppedBacklogFrames; break;
                case "startedAtElapsedMs": values[i] = ReceiverService.startedAtElapsedMs; break;
                case "error": values[i] = ReceiverService.startFailure; break;
                default: throw new IllegalArgumentException("Unsupported receiver status column");
            }
        }
        result.addRow(values);
        return result;
    }

    private Cursor queryInterfaces(String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        if (selection != null || selectionArgs != null || sortOrder != null) throw new IllegalArgumentException("Unsupported interface query");
        String[] columns = projection == null ? INTERFACE_COLUMNS : projection;
        for (String column : columns) if (!contains(INTERFACE_COLUMNS, column)) throw new IllegalArgumentException("Unsupported interface column");
        MatrixCursor result = new MatrixCursor(columns);
        try {
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces != null && interfaces.hasMoreElements()) {
                NetworkInterface network = interfaces.nextElement();
                if (!network.isUp() || network.isLoopback()) continue;
                for (java.net.InterfaceAddress item : network.getInterfaceAddresses()) {
                    if (!(item.getAddress() instanceof Inet4Address) || !item.getAddress().isSiteLocalAddress()) continue;
                    Object[] row = new Object[columns.length];
                    for (int i = 0; i < columns.length; i++) switch (columns[i]) {
                        case "address": row[i] = item.getAddress().getHostAddress(); break;
                        case "prefixLength": row[i] = item.getNetworkPrefixLength(); break;
                        case "interfaceIndex": row[i] = network.getIndex(); break;
                        case "interfaceName": row[i] = network.getName(); break;
                        default: throw new IllegalArgumentException("Unsupported interface column");
                    }
                    result.addRow(row);
                }
            }
        } catch (SocketException error) { throw new IllegalStateException("Receiver interface status is unavailable"); }
        return result;
    }

    private Cursor queryNetwork(String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        if (selection != null || selectionArgs != null || sortOrder != null) throw new IllegalArgumentException("Unsupported network status query");
        String[] columns = projection == null ? NETWORK_COLUMNS : projection;
        for (String column : columns) if (!contains(NETWORK_COLUMNS, column)) throw new IllegalArgumentException("Unsupported network status column");
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) switch (columns[i]) {
            case "networkAudioVersion": row[i] = 1; break;
            case "networkAudioState": row[i] = ReceiverService.networkAudioState; break;
            case "networkAudioFailure": row[i] = ReceiverService.networkAudioFailure; break;
            case "pcmBytesWritten": row[i] = ReceiverService.PCM_BYTES_WRITTEN.get(); break;
            case "networkAudioAddress": row[i] = ReceiverService.networkAudioAddress; break;
            case "networkAudioPort": row[i] = ReceiverService.networkAudioPort; break;
            default: throw new IllegalArgumentException("Unsupported network status column");
        }
        MatrixCursor result = new MatrixCursor(columns, 1);
        result.addRow(row);
        return result;
    }

    private static boolean isPath(Uri uri, String path) {
        return "app.anpod.receiver.state".equals(uri.getAuthority()) && uri.getPathSegments().size() == 1
                && path.equals(uri.getPathSegments().get(0));
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        if (!BINDING_URI.equals(uri.toString()) || !"w".equals(mode))
            throw new FileNotFoundException("Unsupported receiver binding write");
        synchronized (bindingStateLock) {
            if (!bindingWriteInProgress.compareAndSet(false, true))
                throw new FileNotFoundException("A receiver binding write is already in progress");
            bindingFailure = "";
        }
        if (signal != null && signal.isCanceled()) {
            finishBindingWrite("cancelled");
            throw new FileNotFoundException("Receiver binding write was cancelled");
        }

        ParcelFileDescriptor[] pipe = null;
        try {
            pipe = ParcelFileDescriptor.createReliablePipe();
            final ParcelFileDescriptor readEnd = pipe[0];
            final long deadline = SystemClock.elapsedRealtime() + BINDING_READ_TIMEOUT_MS;
            Thread worker = new Thread(() -> {
                String failure = "";
                try (ParcelFileDescriptor ownedReadEnd = readEnd;
                        PairingProtocol.BindingMaterial material = readBindingMaterial(
                                ownedReadEnd, signal, deadline)) {
                    checkCancellation(signal);
                    bindingStore.createOrConfirm(material);
                    bindingFailure = "";
                } catch (BindingInputException error) {
                    failure = error.code;
                } catch (BindingStore.ConflictException error) {
                    failure = "conflict";
                } catch (PairingProtocol.Failure error) {
                    failure = "invalid_record";
                } catch (IOException | RuntimeException error) {
                    failure = "storage_error";
                } finally {
                    finishBindingWrite(failure);
                }
            }, "AnPod-binding-write");
            worker.setDaemon(true);
            worker.start();
            return pipe[1];
        } catch (IOException | RuntimeException error) {
            if (pipe != null) {
                try { pipe[0].close(); } catch (IOException ignored) {}
                try { pipe[1].close(); } catch (IOException ignored) {}
            }
            finishBindingWrite("pipe_error");
            throw new FileNotFoundException("Unable to open receiver binding pipe");
        }
    }

    private Cursor queryBinding() {
        try {
            BindingStore.Status stored = bindingStore.status();
            String state;
            String failure;
            synchronized (bindingStateLock) {
                state = bindingWriteInProgress.get() ? "pending" : stored.state;
                failure = bindingFailure;
            }
            if ("empty".equals(state) && !failure.isEmpty()) state = "failed";
            MatrixCursor result = new MatrixCursor(BINDING_COLUMNS, 1);
            result.addRow(new Object[] {state, stored.commitment, failure});
            return result;
        } catch (IOException | PairingProtocol.Failure error) {
            throw new IllegalStateException("Receiver binding status is unavailable");
        }
    }

    @Override
    public String getType(Uri uri) {
        return "vnd.android.cursor.item/vnd.app.anpod.receiver.health";
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("Read-only status provider");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("Read-only status provider");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("Read-only status provider");
    }

    private static boolean contains(String[] values, String value) {
        for (String candidate : values) if (candidate.equals(value)) return true;
        return false;
    }

    private static boolean sameColumns(String[] actual, String[] expected) {
        if (actual.length != expected.length) return false;
        for (int i = 0; i < actual.length; i++) if (!expected[i].equals(actual[i])) return false;
        return true;
    }

    private static String metric(int value) {
        return value < 0 ? "unknown" : Integer.toString(value);
    }

    private static byte[] readBindingRecord(ParcelFileDescriptor pipe, CancellationSignal signal,
            long deadline) throws BindingInputException, PairingProtocol.Failure {
        byte[] record = new byte[69];
        FileDescriptor fd = pipe.getFileDescriptor();
        try {
            int offset = 0;
            while (offset < record.length) {
                int count = readAvailable(fd, record, offset, record.length - offset, signal, deadline);
                if (count < 0) throw new BindingInputException("invalid_record");
                offset += count;
            }
            byte[] extra = new byte[1];
            try {
                if (readAvailable(fd, extra, 0, 1, signal, deadline) != -1)
                    throw new BindingInputException("invalid_record");
            } finally {
                Arrays.fill(extra, (byte) 0);
            }
            try { pipe.checkError(); }
            catch (IOException error) { throw new BindingInputException("pipe_error"); }
            checkCancellation(signal);
            byte[] result = record;
            record = null;
            return result;
        } finally {
            if (record != null) Arrays.fill(record, (byte) 0);
        }
    }

    private static PairingProtocol.BindingMaterial readBindingMaterial(ParcelFileDescriptor pipe,
            CancellationSignal signal, long deadline) throws BindingInputException, PairingProtocol.Failure {
        byte[] record = readBindingRecord(pipe, signal, deadline);
        try { return PairingProtocol.BindingMaterial.decode(record); }
        finally { Arrays.fill(record, (byte) 0); }
    }

    private static int readAvailable(FileDescriptor fd, byte[] bytes, int offset, int length,
            CancellationSignal signal, long deadline) throws BindingInputException {
        while (true) {
            checkCancellation(signal);
            long remaining = deadline - SystemClock.elapsedRealtime();
            if (remaining <= 0) throw new BindingInputException("timeout");
            StructPollfd pollfd = new StructPollfd();
            pollfd.fd = fd;
            pollfd.events = (short) (OsConstants.POLLIN | OsConstants.POLLHUP);
            try {
                Os.poll(new StructPollfd[] {pollfd}, (int) Math.min(remaining, POLL_SLICE_MS));
                if ((pollfd.revents & (OsConstants.POLLERR | OsConstants.POLLNVAL)) != 0)
                    throw new BindingInputException("pipe_error");
                if ((pollfd.revents & (OsConstants.POLLIN | OsConstants.POLLHUP)) == 0) continue;
                int count = Os.read(fd, bytes, offset, length);
                return count == 0 ? -1 : count;
            } catch (InterruptedIOException error) {
                continue;
            } catch (ErrnoException error) {
                if (error.errno == OsConstants.EINTR) continue;
                throw new BindingInputException("pipe_error");
            }
        }
    }

    private static void checkCancellation(CancellationSignal signal) throws BindingInputException {
        if (signal != null && signal.isCanceled()) throw new BindingInputException("cancelled");
    }

    private void finishBindingWrite(String failure) {
        synchronized (bindingStateLock) {
            bindingFailure = failure;
            bindingWriteInProgress.set(false);
        }
    }

    private static final class BindingInputException extends IOException {
        final String code;
        BindingInputException(String code) { super(code); this.code = code; }
    }
}
