package com.aa4c.desktop

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // 设备发现依赖 mDNS 组播；Android 默认过滤组播包以省电，
  // 必须持有 MulticastLock 才能收发（API_DESIGN.md §11）。
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    acquireMulticastLock()
  }

  override fun onDestroy() {
    releaseMulticastLock()
    super.onDestroy()
  }

  private fun acquireMulticastLock() {
    if (multicastLock != null) return
    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    multicastLock = wifi.createMulticastLock("aa4c-mdns").apply {
      setReferenceCounted(false)
      acquire()
    }
  }

  private fun releaseMulticastLock() {
    multicastLock?.let { if (it.isHeld) it.release() }
    multicastLock = null
  }
}
