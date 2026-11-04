package com.flow_like.app

import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

abstract class NativeFlowTileService : TileService() {
    protected abstract val kind: String
    protected abstract val title: String
    override fun onStartListening() {
        super.onStartListening()
        val available = try { BotNativeStore(this).snapshot() != null } catch (_: Exception) { false }
        qsTile?.apply { label = title; state = if (available) Tile.STATE_INACTIVE else Tile.STATE_UNAVAILABLE; updateTile() }
    }
    override fun onClick() {
        super.onClick()
        unlockAndRun {
            try {
                val snapshot = BotNativeStore(this).snapshot()
                val intent = if (snapshot == null) Intent(this, MainActivity::class.java)
                    else if (kind in listOf("bot", "event")) Intent(this, NativeSearchActivity::class.java).putExtra("selection_kind", kind)
                    else NativeSurfaceLauncher.intent(this, NativeSurfaceChoice(kind, title = title), snapshot.getString("scope"))
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                if (Build.VERSION.SDK_INT >= 34) startActivityAndCollapse(PendingIntent.getActivity(this, kind.hashCode(), intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
                else { @Suppress("DEPRECATION") startActivityAndCollapse(intent) }
            } catch (_: Exception) { }
        }
    }
}

class FlowBotTileService : NativeFlowTileService() { override val kind = "bot"; override val title = "Bots" }
class FlowEventTileService : NativeFlowTileService() { override val kind = "event"; override val title = "Events" }
class FlowPilotTileService : NativeFlowTileService() { override val kind = "flowpilot"; override val title = "Ask FlowPilot" }
class FlowInboxTileService : NativeFlowTileService() { override val kind = "inbox"; override val title = "Notifications" }
