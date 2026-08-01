# Keep the WorkManager Worker + its JNI native method (R061). The Worker is
# instantiated by reflection (WorkManager), and nativeSync is resolved by name
# from Kotlin.
-keep class xyz.yzx9.gpm.backgroundsync.SyncWorker { *; }
-keep class xyz.yzx9.gpm.backgroundsync.MasterKeyAccess { *; }
