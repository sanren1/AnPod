package app.anpod.receiver;

import android.app.ActivityManager;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.content.pm.ServiceInfo;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.concurrent.atomic.AtomicBoolean;

public final class ReceiverStateProvider extends ContentProvider {
    private static final String BINDING_URI = "content://app.anpod.receiver.state/binding";
    private static final String[] BINDING_COLUMNS = {"bindingState", "bindingCommitment"};
    private static final String[] COLUMNS = {
            "applicationId", "versionName", "versionCode", "serviceActive",
            "foreground", "healthy", "audioActive", "actualMode", "actualSampleRate",
            "bufferCapacityFrames", "effectiveBufferFrames", "startThresholdFrames", "underrunCount",
            "startedAtElapsedMs", "error"
    };
    private final AtomicBoolean bindingWriteInProgress = new AtomicBoolean();
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
        if (!BINDING_URI.equals(uri.toString())
                && (!"app.anpod.receiver.state".equals(uri.getAuthority())
                || uri.getPathSegments().size() != 1
                || !"health".equals(uri.getPathSegments().get(0)))) {
            throw new IllegalArgumentException("Unsupported receiver status URI");
        }

        if (BINDING_URI.equals(uri.toString())) {
            if (projection == null || !sameColumns(projection, BINDING_COLUMNS)
                    || selection != null || selectionArgs != null || sortOrder != null)
                throw new IllegalArgumentException("Unsupported receiver binding query");
            return queryBinding();
        }

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
                case "startedAtElapsedMs": values[i] = ReceiverService.startedAtElapsedMs; break;
                case "error": values[i] = ReceiverService.startFailure; break;
                default: throw new IllegalArgumentException("Unsupported receiver status column");
            }
        }
        result.addRow(values);
        return result;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        if (!BINDING_URI.equals(uri.toString()) || !"w".equals(mode))
            throw new FileNotFoundException("Unsupported receiver binding write");
        if (!bindingWriteInProgress.compareAndSet(false, true))
            throw new FileNotFoundException("A receiver binding write is already in progress");

        ParcelFileDescriptor[] pipe = null;
        try {
            pipe = ParcelFileDescriptor.createReliablePipe();
            final ParcelFileDescriptor readEnd = pipe[0];
            Thread worker = new Thread(() -> {
                try (ParcelFileDescriptor.AutoCloseInputStream input =
                        new ParcelFileDescriptor.AutoCloseInputStream(readEnd);
                        PairingProtocol.BindingMaterial material = PairingProtocol.BindingMaterial.readRecord(input)) {
                    bindingStore.createOrConfirm(material);
                } catch (IOException | PairingProtocol.Failure | RuntimeException ignored) {
                    // The caller confirms the persisted binding through the read-only status URI.
                } finally {
                    bindingWriteInProgress.set(false);
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
            bindingWriteInProgress.set(false);
            throw new FileNotFoundException("Unable to open receiver binding pipe");
        }
    }

    private Cursor queryBinding() {
        try {
            BindingStore.Status stored = bindingStore.status();
            String state = bindingWriteInProgress.get() ? "pending" : stored.state;
            MatrixCursor result = new MatrixCursor(BINDING_COLUMNS, 1);
            result.addRow(new Object[] {state, stored.commitment});
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
}
