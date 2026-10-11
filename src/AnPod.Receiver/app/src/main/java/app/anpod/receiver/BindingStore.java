package app.anpod.receiver;

import android.util.AtomicFile;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.Arrays;

/** Create-only binding storage under an explicitly supplied app-private files directory. */
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

    void createIfAbsent(PairingProtocol.BindingMaterial material) throws IOException {
        synchronized (FILE_LOCK) {
            File backup = new File(bindingFile.getPath() + ".bak");
            File pending = new File(bindingFile.getPath() + ".new");
            if (bindingFile.exists() || backup.exists() || pending.exists())
                throw new IOException("AnPod binding already exists or needs recovery");

            byte[] record = material.encodedRecord();
            FileOutputStream output = null;
            try {
                output = atomicFile.startWrite();
                output.write(record);
                atomicFile.finishWrite(output);
                output = null;
            } catch (IOException | RuntimeException error) {
                if (output != null) atomicFile.failWrite(output);
                throw error;
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
}
