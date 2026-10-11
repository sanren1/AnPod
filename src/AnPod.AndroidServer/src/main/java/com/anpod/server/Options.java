package com.anpod.server;

/**
 * Command line options passed by AnPod when launching the payload with
 * {@code app_process}.
 *
 * <p>Derived from ysbing/AudioShare server Options.java (LGPL-3.0-or-later).
 * AnPod adds length limits so a malformed argument cannot be echoed back as an
 * oversized handshake.
 */
public final class Options {

    private static final int MAX_SOCKET_NAME_LENGTH = 128;
    private static final int MAX_CONNECT_CODE_LENGTH = 128;

    final String socketName;
    final String connectCode;
    final String playbackMode;

    private Options(String socketName, String connectCode, String playbackMode) {
        this.socketName = socketName;
        this.connectCode = connectCode;
        this.playbackMode = playbackMode;
    }

    public static Options parse(String... args) {
        String socketName = null;
        String connectCode = null;
        String playbackMode = "balanced";

        for (String arg : args) {
            if (arg == null) {
                continue;
            }

            int equalIndex = arg.indexOf('=');
            if (equalIndex <= 0) {
                throw new IllegalArgumentException("ARGUMENT_FORMAT_INVALID");
            }

            String key = arg.substring(0, equalIndex);
            String value = arg.substring(equalIndex + 1);

            if ("socketName".equals(key)) {
                socketName = value;
            } else if ("connectCode".equals(key)) {
                connectCode = value;
            } else if ("playbackMode".equals(key)) {
                playbackMode = value;
            }
        }

        if (socketName == null || socketName.isEmpty()) {
            throw new IllegalArgumentException("SOCKET_NAME_REQUIRED");
        }

        if (connectCode == null) {
            throw new IllegalArgumentException("CONNECT_CODE_REQUIRED");
        }

        if (socketName.length() > MAX_SOCKET_NAME_LENGTH) {
            throw new IllegalArgumentException("SOCKET_NAME_TOO_LONG");
        }

        if (connectCode.length() > MAX_CONNECT_CODE_LENGTH) {
            throw new IllegalArgumentException("CONNECT_CODE_TOO_LONG");
        }

        if (!"balanced".equals(playbackMode) && !"low-latency".equals(playbackMode)) {
            throw new IllegalArgumentException("PLAYBACK_MODE_INVALID");
        }

        return new Options(socketName, connectCode, playbackMode);
    }
}
