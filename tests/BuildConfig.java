package app.anpod.receiver;

/**
 * Compile-time stand-in for the Gradle-generated BuildConfig.
 *
 * <p>The P5-C loopback harness compiles the receiver sources with plain javac, outside Gradle, so it
 * supplies this shim with test hooks enabled. Gradle builds always use the generated class.
 */
public final class BuildConfig {
    public static final boolean DEBUG = true;

    private BuildConfig() { }
}
