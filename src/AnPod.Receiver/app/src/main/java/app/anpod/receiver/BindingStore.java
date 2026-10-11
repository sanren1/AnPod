package app.anpod.receiver;

import android.util.AtomicFile;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.security.MessageDigest;
import java.util.Arrays;

/** Create-only binding storage under the Receiver's fixed app-private files directory. */
final class BindingStore {
    private static final Object FILE_LOCK = new Object();
    private final File bindingFile;
    private final AtomicFile atomicFile;

    BindingStore(File applicationPrivateFilesDirectory) {
        if (applicationPrivateFilesDirectory == null || !applicationPrivateFilesDirectory.isDirectory())
            throw new IllegalArgumentException("an existing app-private files directory is required");
        bindingFile = new File(applicationPrivateFilesDirectory, "anpod-binding-v1.bin");
        atomicFile = new AtomicFile(bindingFile);
    }

    void createOrConfirm(PairingProtocol.BindingMaterial material) throws IOException, PairingProtocol.Failure {
        synchronized (FILE_LOCK) {
            byte[] record = material.encodedRecord();
            try {
                File backup = new File(bindingFile.getPath() + ".bak");
                if (bindingFile.exists() || backup.exists()) {
                    byte[] existing = atomicFile.readFully();
                    try (PairingProtocol.BindingMaterial decoded = PairingProtocol.BindingMaterial.decode(existing)) {
                        byte[] existingRecord = decoded.encodedRecord();
                        try {
                            if (!MessageDigest.isEqual(record, existingRecord))
                                throw new IOException("AnPod binding conflict");
                            return;
                        } finally {
                            Arrays.fill(existingRecord, (byte) 0);
                        }
                    } finally {
                        Arrays.fill(existing, (byte) 0);
                    }
                }

                FileOutputStream output = null;
                try {
                    output = atomicFile.startWrite();
                    output.write(record);
                    atomicFile.finishWrite(output);
                    output = null;
                } catch (IOException | RuntimeException error) {
                    if (output != null) atomicFile.failWrite(output);
                    throw error;
                }
            } finally {
                Arrays.fill(record, (byte) 0);
            }
        }
    }

    PairingProtocol.BindingMaterial load() throws IOException, PairingProtocol.Failure {
        synchronized (FILE_LOCK) {
            byte[] record = atomicFile.readFully();
            try { return PairingProtocol.BindingMaterial.decode(record); }
            finally { Arrays.fill(record, (byte) 0); }
        }
    }

    Status status() throws IOException, PairingProtocol.Failure {
        synchronized (FILE_LOCK) {
            File backup = new File(bindingFile.getPath() + ".bak");
            if (!bindingFile.exists() && !backup.exists()) return new Status("empty", "");
            byte[] record = atomicFile.readFully();
            try (PairingProtocol.BindingMaterial material = PairingProtocol.BindingMaterial.decode(record)) {
                return new Status("saved", hex(material.commitment()));
            } finally {
                Arrays.fill(record, (byte) 0);
            }
        }
    }

    private static String hex(byte[] value) {
        char[] digits = "0123456789abcdef".toCharArray();
        char[] result = new char[value.length * 2];
        for (int i = 0; i < value.length; i++) {
            result[i * 2] = digits[(value[i] >>> 4) & 0x0f];
            result[i * 2 + 1] = digits[value[i] & 0x0f];
        }
        Arrays.fill(value, (byte) 0);
        return new String(result);
    }

    static final class Status {
        final String state;
        final String commitment;

        Status(String state, String commitment) {
            this.state = state;
            this.commitment = commitment;
        }
    }
}
