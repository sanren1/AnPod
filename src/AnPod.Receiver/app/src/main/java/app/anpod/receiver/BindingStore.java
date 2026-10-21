package app.anpod.receiver;

import android.util.AtomicFile;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;

/** Create-only binding storage under the Receiver's fixed app-private files directory. */
final class BindingStore implements BackgroundStartAuthorization.GrantStore {
    private static final Object FILE_LOCK = new Object();
    private final File bindingFile;
    private final AtomicFile atomicFile;
    private final AtomicFile grantFile;

    enum ResetResult { REMOVED, ALREADY_EMPTY }

    static Object storageLock() { return FILE_LOCK; }

    BindingStore(File applicationPrivateFilesDirectory) {
        if (applicationPrivateFilesDirectory == null || !applicationPrivateFilesDirectory.isDirectory())
            throw new IllegalArgumentException("an existing app-private files directory is required");
        bindingFile = new File(applicationPrivateFilesDirectory, "anpod-binding-v1.bin");
        atomicFile = new AtomicFile(bindingFile);
        grantFile = new AtomicFile(new File(applicationPrivateFilesDirectory, "anpod-background-grant-v1.bin"));
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
                                throw new ConflictException();
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

    ResetResult removeIfMatches(String expectedCommitment) throws IOException, PairingProtocol.Failure {
        synchronized (FILE_LOCK) {
            File backup = new File(bindingFile.getPath() + ".bak");
            if (!bindingFile.exists() && !backup.exists()) {
                if (expectedCommitment == null) return ResetResult.ALREADY_EMPTY;
                throw new ConflictException();
            }
            byte[] record = atomicFile.readFully();
            try (PairingProtocol.BindingMaterial material = PairingProtocol.BindingMaterial.decode(record)) {
                if (expectedCommitment == null || !hex(material.commitment()).equals(expectedCommitment))
                    throw new ConflictException();
                clearUntilRevoked(expectedCommitment);
                atomicFile.delete();
                if (bindingFile.exists() || backup.exists()) throw new IOException("binding remains after reset");
                return ResetResult.REMOVED;
            } finally {
                Arrays.fill(record, (byte) 0);
            }
        }
    }

    @Override public boolean hasUntilRevoked(String expectedCommitment) throws IOException {
        synchronized (FILE_LOCK) {
            if (!grantFile.getBaseFile().exists() && !new File(grantFile.getBaseFile().getPath() + ".bak").exists()) return false;
            byte[] record = grantFile.readFully();
            try {
                String value = new String(record, StandardCharsets.US_ASCII);
                String prefix = "BSA1\n";
                if (!value.startsWith(prefix) || !value.endsWith("\n")
                        || !isCommitment(value.substring(prefix.length(), value.length() - 1)))
                    throw new IOException("invalid background grant record");
                return (prefix + expectedCommitment + "\n").equals(value);
            }
            finally { Arrays.fill(record, (byte) 0); }
        }
    }

    @Override public void saveUntilRevoked(String commitment) throws IOException {
        synchronized (FILE_LOCK) {
            byte[] record = ("BSA1\n" + commitment + "\n").getBytes(StandardCharsets.US_ASCII);
            FileOutputStream output = null;
            try {
                output = grantFile.startWrite();
                output.write(record);
                grantFile.finishWrite(output);
                output = null;
            } catch (IOException | RuntimeException error) {
                if (output != null) grantFile.failWrite(output);
                throw error;
            } finally { Arrays.fill(record, (byte) 0); }
        }
    }

    @Override public void clearUntilRevoked(String expectedCommitment) throws IOException {
        synchronized (FILE_LOCK) {
            File base = grantFile.getBaseFile();
            File backup = new File(base.getPath() + ".bak");
            if (!base.exists() && !backup.exists()) return;
            byte[] record = grantFile.readFully();
            try {
                String value = new String(record, StandardCharsets.US_ASCII);
                if (value.equals("BSA1\n" + expectedCommitment + "\n")) {
                    grantFile.delete();
                    if (hasUntilRevoked(expectedCommitment))
                        throw new IOException("background grant remains after revoke");
                }
            } finally { Arrays.fill(record, (byte) 0); }
        }
    }

    private static boolean isCommitment(String value) {
        if (value.length() != 64) return false;
        for (int index = 0; index < value.length(); index++) {
            char item = value.charAt(index);
            if (!(item >= '0' && item <= '9') && !(item >= 'a' && item <= 'f')) return false;
        }
        return true;
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

    static final class ConflictException extends IOException {
        ConflictException() { super("AnPod binding conflict"); }
    }
}
