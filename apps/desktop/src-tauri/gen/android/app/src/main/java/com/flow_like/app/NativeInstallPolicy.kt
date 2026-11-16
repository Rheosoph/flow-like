package com.flow_like.app

internal enum class NativeInstallPhase {
    WAITING, CONFIRMATION, DOWNLOADING, INSTALLING, CANCELING, CANCELED, FAILED, INSTALLED, UNKNOWN;

    val active: Boolean
        get() = this in setOf(WAITING, CONFIRMATION, DOWNLOADING, INSTALLING, CANCELING)
}

internal data class NativeInstallSession(
    val id: Int,
    val modules: Set<String>,
    val phase: NativeInstallPhase,
)

internal object NativeInstallPolicy {
    val modules: Set<String> = linkedSetOf("runtime", "editors")

    fun missingModules(playDelivery: Boolean, installed: Set<String>, markers: Set<String>): Set<String> =
        if (playDelivery) modules - installed - markers else emptySet()

    fun selectSession(sessions: List<NativeInstallSession>, rememberedId: Int): NativeInstallSession? {
        val relevant = sessions.filter { session -> session.modules.any { it in modules } }
        return relevant.firstOrNull { it.id == rememberedId }
            ?: relevant.filter { it.phase.active }.maxByOrNull { it.id }
    }

    fun progress(downloaded: Long, total: Long): Int? = if (total <= 0) null
        else ((downloaded.coerceAtLeast(0).toDouble() / total) * 100).toInt().coerceIn(0, 100)
}
