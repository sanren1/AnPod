package app.anpod.receiver

/** Private notification entry that reuses MainActivity's normal foreground checks. */
class RecoveryActivity : MainActivity() {
    companion object {
        const val EXTRA_BINDING_COMMITMENT = "bootStandbyBindingCommitment"
        const val EXTRA_OPERATION_ID = "bootStandbyOperationId"
    }
}
