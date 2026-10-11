package app.anpod.receiver;

import android.app.ActivityManager;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.content.pm.ServiceInfo;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;

public final class ReceiverStateProvider extends ContentProvider {
    private static final String[] COLUMNS = {
            "applicationId", "versionName", "versionCode", "serviceActive",
            "foreground", "healthy", "audioActive", "actualMode", "startedAtElapsedMs", "error"
    };

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection,
            String[] selectionArgs, String sortOrder) {
        if (!"app.anpod.receiver.state".equals(uri.getAuthority())
                || uri.getPathSegments().size() != 1
                || !"health".equals(uri.getPathSegments().get(0))) {
            throw new IllegalArgumentException("Unsupported receiver status URI");
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
        boolean healthy = active && foreground && ReceiverService.foregroundReady;
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
                case "startedAtElapsedMs": values[i] = ReceiverService.startedAtElapsedMs; break;
                case "error": values[i] = ReceiverService.startFailure; break;
                default: throw new IllegalArgumentException("Unsupported receiver status column");
            }
        }
        result.addRow(values);
        return result;
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
}
