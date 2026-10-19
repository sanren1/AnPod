package app.anpod.aoaprobe;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.Intent;
import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;
import android.os.Build;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;

/** Standalone lab app, NOT the AnPod Receiver. No audio, network, or system permissions. */
public final class MainActivity extends Activity {
    private static final String ACTION_PERMISSION = "app.anpod.aoaprobe.USB_PERMISSION";
    private static final int BLOCK = 4096;
    private UsbManager usbManager;
    private TextView output;
    private final Object lock = new Object();
    private ParcelFileDescriptor descriptor;
    private Thread worker;
    private int lastSeq;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        usbManager = (UsbManager)getSystemService(USB_SERVICE);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        LinearLayout root = new LinearLayout(this);
        root.setPadding(32, 40, 32, 24);
        root.setOrientation(LinearLayout.VERTICAL);
        TextView heading = new TextView(this);
        heading.setText("AnPod AOA Bulk Probe\nStandalone USB test (no audio)");
        heading.setTextSize(21);
        root.addView(heading);
        Button connect = new Button(this);
        connect.setText("Find accessory / request permission / connect");
        connect.setOnClickListener(v -> tryConnect());
        root.addView(connect);
        Button close = new Button(this);
        close.setText("Close USB stream");
        close.setOnClickListener(v -> closeConnection());
        root.addView(close);
        output = new TextView(this);
        output.setTextIsSelectable(true);
        output.setTextSize(14);
        output.setText("Probe installed. Now use Windows -Mode Switch.\n");
        ScrollView scroll = new ScrollView(this);
        scroll.addView(output);
        root.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        setContentView(root);
        if (ACTION_PERMISSION.equals(getIntent().getAction())) {
            Boolean granted = getIntent().getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false);
            log("USB permission result: " + granted);
        }
        tryConnect();
    }
    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (ACTION_PERMISSION.equals(intent.getAction())) {
            log("USB permission result: " + intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false));
        }
        tryConnect();
    }
    private void log(String line) {
        runOnUiThread(() -> {
            if (output != null) {
                CharSequence old = output.getText();
                if (old.length() > 14000) old = old.subSequence(old.length() - 9000, old.length());
                output.setText(old + "\n" + line);
            }
        });
    }
    private void tryConnect() {
        synchronized (lock) { if (descriptor != null) { log("Already connected; wait for host Bulk mode.");return; } }
        UsbAccessory[] list = usbManager.getAccessoryList();
        if (list == null || list.length == 0) { log("No accessory. Install this App first; switch from PC next.");return; }
        UsbAccessory accessory = null;
        for (UsbAccessory a : list) {
            if ("AnPod Lab".equals(a.getManufacturer()) && "AOA Bulk Probe".equals(a.getModel())) {
                accessory = a; break;
            }
        }
        if (accessory == null) { log("Connected accessory does not match AnPod Lab / AOA Bulk Probe. Ignoring.");return; }
        if (!usbManager.hasPermission(accessory)) {
            log("Requesting Android permission for accessory...");
            Intent callback = new Intent(this, MainActivity.class).setAction(ACTION_PERMISSION)
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
            int flags = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= 31) flags |= PendingIntent.FLAG_MUTABLE;
            PendingIntent pi = PendingIntent.getActivity(this, 0, callback, flags);
            usbManager.requestPermission(accessory, pi);
            return;
        }
        try {
            ParcelFileDescriptor opened = usbManager.openAccessory(accessory);
            if (opened == null) { log("openAccessory returned null.");return; }
            synchronized (lock) {
                if (descriptor != null) { opened.close();return; }
                descriptor = opened; lastSeq = 0;
            }
            worker = new Thread(() -> echoLoop(opened), "AnPod AOA echo worker");
            worker.setDaemon(true);
            worker.start();
            log("OPEN: USB accessory stream ready. Now run Windows -Mode Bulk.");
        } catch (Exception e) { log("openAccessory error: " + e); }
    }
    private void echoLoop(ParcelFileDescriptor owned) {
        int received = 0;
        try (FileInputStream in = new FileInputStream(owned.getFileDescriptor());
             FileOutputStream out = new FileOutputStream(owned.getFileDescriptor())) {
            byte[] block = new byte[BLOCK];
            for (;;) {
                if (!readExactly(in, block)) { log("USB EOF/disconnect");return; }
                int seq = check(block);
                if (seq <= 0) throw new IOException("Invalid block header / checksum / sequence");
                out.write(block);
                out.flush();
                received++;
                lastSeq = seq;
                if (received == 1 || received % 64 == 0) log("Echo OK: " + received + " blocks; last seq=" + lastSeq);
            }
        } catch (IOException e) { log("USB worker ended: " + e.getMessage()); }
        finally {
            synchronized (lock) {
                if (descriptor == owned) descriptor = null;
            }
            try { owned.close(); } catch (IOException ignored) { }
            log("USB stream closed. Reconnect or unplug/replug if needed.");
        }
    }
    private static boolean readExactly(FileInputStream in, byte[] target) throws IOException {
        int offset = 0;
        while (offset < target.length) {
            int n = in.read(target, offset, target.length - offset);
            if (n < 0) {if(offset==0) return false;throw new IOException("Partial frame before EOF");}
            if (n == 0) continue;
            offset += n;
        }
        return true;
    }
    private static int check(byte[] b) {
        if (b.length != BLOCK || b[0] != 'A' || b[1] != 'P' || b[2] != 'A' || b[3] != 'O'
                || b[4] != 1 || b[5] != 1 || b[6] != 0 || b[7] != 0
                || (b[12] & 255) != 0 || (b[13] & 255) != 16 || b[14] != 0 || b[15] != 0)
            return -1;
        int seq = (b[8] & 255) | ((b[9] & 255) << 8) | ((b[10] & 255) << 16) | ((b[11] & 255) << 24);
        if (seq <= 0 || seq > 512) return -1;
        for (int i = 16; i < b.length; i++)
            if (b[i] != (byte)((seq * 131 + i * 17 + 7) & 255)) return -1;
        return seq;
    }
    private void closeConnection() {
        ParcelFileDescriptor previous;
        synchronized (lock) { previous = descriptor; descriptor = null; }
        if (previous != null) {try {previous.close();}catch(IOException ignored){}}
        log("Close requested.");
    }
    @Override protected void onDestroy() { closeConnection(); super.onDestroy(); }
}
