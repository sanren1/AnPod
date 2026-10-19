package app.anpod.audioprobe;

import android.app.Activity;
import android.content.Intent;
import android.media.AudioManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Spinner;
import android.widget.ArrayAdapter;
import android.widget.TextView;
import org.json.JSONObject;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Date;
import java.util.Locale;

/** Standalone output-engine benchmark. No networking, USB, audio recording, or AnPod integration. */
public final class MainActivity extends Activity {
    static { System.loadLibrary("anpod_audio_probe"); }
    private static native String nativeStart(short[] clip, int sampleRate, int channels, int bursts);
    private static native void nativeStop();
    private static native String nativeSnapshot();

    private static final int SAMPLE_RATE = 48000;
    private static final int CHANNELS = 2;
    private static final int SAVE_CSV_REQUEST = 4301;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final AudioTrackEngine javaEngine = new AudioTrackEngine();
    private final ArrayList<String> csvRows = new ArrayList<>();
    private final ArrayList<Double> validLatencies = new ArrayList<>();
    private short[] clip;
    private TextView status;
    private TextView summary;
    private Spinner bufferSpinner;
    private String currentEngine = "";
    private boolean playing;
    private long startedAt;
    private String exportText;
    private int audioBurstHint = 192;
    private int previousXrun = -1;
    private long previousCallbackCount;
    private String lastError = "";
    private final Runnable tick = new Runnable() {
        @Override public void run() {
            if (!playing) return;
            try { updateTelemetry(); }
            catch (Exception error) { lastError = error.toString(); status.setText("采样失败: " + lastError); }
            handler.postDelayed(this, 1000);
        }
    };

    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        clip = createTestClip();
        AudioManager am = (AudioManager) getSystemService(AUDIO_SERVICE);
        String prop = am.getProperty(AudioManager.PROPERTY_OUTPUT_FRAMES_PER_BUFFER);
        try { int n = Integer.parseInt(prop); if (n > 0 && n < 4096) audioBurstHint = n; }
        catch (Exception ignored) { }
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        buildScreen();
        csvRows.add("engine,elapsed_s,requested_bursts,rate,channels,api,sharing,mmap,perf,"
                + "burst_frames,buffer_frames,capacity_frames,xruns,xrun_delta,latency_est_ms,"
                + "callback_or_write_calls,callback_gap_ms,max_callback_gap_or_write_ms,frames_written,warmup");
    }

    private void buildScreen() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(dp(18), dp(20), dp(18), dp(24));
        layout.setBackgroundColor(0xFFF7F9FB);
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.addView(layout);
        setContentView(scroll);
        TextView heading = new TextView(this);
        heading.setText("AnPod · 音频引擎对照");
        heading.setTextSize(23);
        heading.setTextColor(0xFF142433);
        layout.addView(heading);
        TextView explanation = new TextView(this);
        explanation.setText("只播放本机合成的相同 PCM16 信号。48kHz · 立体声 · Media · LowLatency。"
                + "AudioTrack 为阻塞写入；Oboe 为 AAudio Shared 回调。无网络、无录音、无需 Root。"
                + "\n\n注意：本工具的时间戳延迟是估算值，不是扬声器声学延迟。"
                + "测试期间保持前台和亮屏，先调低媒体音量。");
        explanation.setTextSize(14);
        explanation.setTextColor(0xFF4B5863);
        explanation.setPadding(0, dp(12), 0, dp(14));
        layout.addView(explanation);

        TextView label = new TextView(this);
        label.setText("目标缓冲：按 Burst 数选择（建议先 2）");
        label.setTextColor(0xFF142433);
        label.setTextSize(15);
        layout.addView(label);
        bufferSpinner = new Spinner(this);
        bufferSpinner.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item,
                new String[]{"2 Bursts", "3 Bursts", "4 Bursts"}));
        layout.addView(bufferSpinner);

        Button javaBtn = addButton(layout, "开始 AudioTrack 基准测试");
        javaBtn.setOnClickListener(v -> begin(false));
        Button oboeBtn = addButton(layout, "开始 Oboe / AAudio 测试");
        oboeBtn.setOnClickListener(v -> begin(true));
        Button stopBtn = addButton(layout, "停止当前测试");
        stopBtn.setOnClickListener(v -> end());
        Button exportBtn = addButton(layout, "导出两组 CSV 结果");
        exportBtn.setOnClickListener(v -> exportCsv());

        summary = new TextView(this);
        summary.setTextSize(15);
        summary.setTextColor(0xFF15222E);
        summary.setPadding(0, dp(12), 0, dp(12));
        summary.setText("请先测试 AudioTrack 约 2 分钟，再测试 Oboe 约 2 分钟。\n"
                + "系统输出缓冲帧提示值：" + audioBurstHint + "（AudioTrack 未提供真实 burst API）。");
        layout.addView(summary);
        status = new TextView(this);
        status.setTextSize(13);
        status.setTextColor(0xFF334451);
        status.setPadding(dp(10), dp(12), dp(10), dp(12));
        status.setBackgroundColor(0xFFE6EDF3);
        status.setTextIsSelectable(true);
        status.setText("尚未开始；每秒采样一次，首 5 秒为预热，不纳入延迟统计。");
        layout.addView(status);
    }

    private Button addButton(LinearLayout parent, String text) {
        Button b = new Button(this);
        b.setText(text);
        b.setAllCaps(false);
        parent.addView(b, new LinearLayout.LayoutParams(-1, dp(52)));
        return b;
    }

    private static short[] createTestClip() {
        // Deterministic signal, exactly identical for both engines. ~0.22 peak at most.
        final int frames = SAMPLE_RATE * 4;
        short[] pcm = new short[frames * CHANNELS];
        for (int i = 0; i < frames; i++) {
            double t = i / (double) SAMPLE_RATE;
            double pulsePhase = t - Math.floor(t);
            double clickEnvelope = pulsePhase < 0.045
                    ? Math.sin(Math.PI * pulsePhase / 0.045) : 0.0;
            double signal = 0.085 * Math.sin(2 * Math.PI * 440 * t)
                    + 0.055 * Math.sin(2 * Math.PI * 660 * t)
                    + 0.075 * clickEnvelope * Math.sin(2 * Math.PI * 1760 * t);
            pcm[2 * i] = (short) Math.max(-32768, Math.min(32767, Math.round(signal * 32767)));
            pcm[2 * i + 1] = (short) Math.max(-32768,
                    Math.min(32767, Math.round(signal * 0.92 * 32767)));
        }
        return pcm;
    }

    private void begin(boolean oboe) {
        if (playing) end();
        final int bursts = bufferSpinner.getSelectedItemPosition() + 2;
        String error;
        try {
            if (oboe) {
                error = nativeStart(clip, SAMPLE_RATE, CHANNELS, bursts);
            } else {
                error = javaEngine.start(clip, bursts, audioBurstHint);
            }
        } catch (Exception e) { error = e.toString(); }
        if (!error.isEmpty()) {
            status.setText("无法开始: " + error);
            return;
        }
        currentEngine = oboe ? "Oboe" : "AudioTrack";
        playing = true;
        startedAt = System.nanoTime();
        previousXrun = -1;
        previousCallbackCount = 0;
        validLatencies.clear();
        lastError = "";
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        bufferSpinner.setEnabled(false);
        handler.removeCallbacks(tick);
        handler.postDelayed(tick, 1000);
        summary.setText("运行中：" + currentEngine + "（目标 " + bursts + " Bursts）");
    }

    private void updateTelemetry() throws Exception {
        JSONObject data = "Oboe".equals(currentEngine)
                ? new JSONObject(nativeSnapshot()) : javaEngine.snapshot();
        long seconds = Math.round((System.nanoTime() - startedAt) / 1e9);
        if (!data.optBoolean("running", false)) {
            status.setText("流已停止（可能是设备断开）。" + data.optString("error"));
            end();
            return;
        }
        int bursts = bufferSpinner.getSelectedItemPosition() + 2;
        int xrun = data.optInt("xrun", -1);
        int delta = (previousXrun >= 0 && xrun >= 0) ? Math.max(0, xrun - previousXrun) : -1;
        previousXrun = xrun;
        double latency = data.optDouble("latencyMs", -1);
        if (seconds > 5 && latency >= 0 && Double.isFinite(latency)) validLatencies.add(latency);
        long cbCount = data.optLong("callbackCount", 0);
        long cbDiff = Math.max(0, cbCount - previousCallbackCount);
        previousCallbackCount = cbCount;

        csvRows.add(csv(currentEngine) + "," + seconds + "," + bursts + ","
                + data.optInt("sampleRate", 0) + "," + data.optInt("channels", 0) + ","
                + csv(data.optString("api")) + "," + csv(data.optString("sharing")) + ","
                + csv(data.optString("mmap")) + "," + csv(data.optString("perf")) + ","
                + data.optInt("framesPerBurst", -1) + "," + data.optInt("bufferFrames", -1) + ","
                + data.optInt("capacityFrames", -1) + "," + xrun + "," + delta + ","
                + metric(latency) + "," + cbCount + ","
                + metric(data.optDouble("callbackGapMs", -1)) + ","
                + metric(data.optDouble("maxCallbackGapMs", -1)) + ","
                + data.optLong("framesWritten", -1) + "," + (seconds <= 5 ? "1" : "0"));

        String text = currentEngine + " | " + seconds + "s | requested " + bursts + " bursts\n"
                + "API: " + data.optString("api") + " | " + data.optString("sharing") + " | MMAP: " + data.optString("mmap")
                + "\nPerf: " + data.optString("perf") + " | " + data.optInt("sampleRate") + "Hz / " + data.optInt("channels") + "ch"
                + "\nBurst: " + data.optInt("framesPerBurst")
                + (data.optBoolean("burstIsHint", false) ? " (系统提示值，非 AudioTrack 实测)" : " (真实输出流)")
                + "\nBuffer: " + data.optInt("bufferFrames") + " / " + data.optInt("capacityFrames") + " frames"
                + "\nLatency estimate: " + metric(latency) + "ms (no acoustic DSP)"
                + "\nXRuns: " + (xrun < 0 ? "unavailable" : xrun) + " | delta=" + delta
                + "\ncallbacks/write calls: " + cbCount + " (+" + cbDiff + "/s)"
                + "\ncallback gap or max blocking write: "
                + metric(data.optDouble("callbackGapMs", -1)) + "ms / "
                + metric(data.optDouble("maxCallbackGapMs", -1)) + "ms"
                + "\n" + percentiles();
        status.setText(text);
        summary.setText("当前：" + currentEngine + " | 采样 " + (csvRows.size() - 1)
                + " 行\n" + percentiles());
    }

    private String percentiles() {
        if (validLatencies.isEmpty()) return "P50 / P95: 等待有效时间戳（预热 5s）";
        ArrayList<Double> sorted = new ArrayList<>(validLatencies);
        Collections.sort(sorted);
        double p50 = sorted.get(Math.min(sorted.size() - 1, (int) Math.ceil(sorted.size() * 0.50) - 1));
        double p95 = sorted.get(Math.min(sorted.size() - 1, (int) Math.ceil(sorted.size() * 0.95) - 1));
        return "时间戳延迟估算 P50=" + metric(p50) + "ms, P95=" + metric(p95)
                + "ms (n=" + sorted.size() + ")";
    }

    private static String metric(double value) {
        return (Double.isFinite(value) && value >= 0) ? String.format(Locale.US, "%.3f", value) : "";
    }

    private static String csv(String s) {
        return "\"" + (s == null ? "" : s.replace("\"", "\"\"")) + "\"";
    }

    private void end() {
        handler.removeCallbacks(tick);
        if (!playing) return;
        playing = false;
        try {
            if ("Oboe".equals(currentEngine)) nativeStop(); else javaEngine.stop();
        } catch (Exception e) { lastError = e.toString(); }
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        bufferSpinner.setEnabled(true);
        summary.setText("已停止 " + currentEngine + "。" + percentiles()
                + (lastError.isEmpty() ? "" : "\n注意: " + lastError)
                + "\n可直接开始另一引擎，最后导出 CSV。");
    }

    private void exportCsv() {
        if (csvRows.size() <= 1) { status.setText("还没有任何采样数据。"); return; }
        exportText = String.join("\n", csvRows) + "\n";
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("text/csv");
        intent.putExtra(Intent.EXTRA_TITLE, "AnPod_Audio_Engine_Probe_"
                + new SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(new Date()) + ".csv");
        startActivityForResult(intent, SAVE_CSV_REQUEST);
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != SAVE_CSV_REQUEST || resultCode != RESULT_OK || data == null) return;
        try (OutputStream out = getContentResolver().openOutputStream(data.getData())) {
            if (out == null) throw new IllegalStateException("No writable document");
            out.write(exportText.getBytes(StandardCharsets.UTF_8));
            out.flush();
            status.setText("已导出 CSV：" + data.getData());
        } catch (Exception error) { status.setText("导出失败：" + error); }
        exportText = null;
    }

    @Override protected void onStop() {
        end(); // No background audio/foreground services, by design.
        super.onStop();
    }

    @Override protected void onDestroy() {
        end();
        handler.removeCallbacks(tick);
        super.onDestroy();
    }

    private int dp(int value) { return (int) (getResources().getDisplayMetrics().density * value + 0.5f); }
}
