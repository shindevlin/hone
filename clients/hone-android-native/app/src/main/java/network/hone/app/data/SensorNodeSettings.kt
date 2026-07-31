package network.hone.app.data

import java.net.URI

/**
 * Explicit, non-secret settings required before sensor submission may be enabled.
 *
 * There are deliberately no account, device, or node defaults. App state must obtain
 * these values from the operator and inject them into the submission flow. Owner key
 * material is not part of this object; enrollment/revocation receive only a short-lived
 * [SensorSigner] callback for the individual operation.
 */
data class SensorNodeSettings(
    val chainId: String,
    val owner: String,
    val nodeUrl: String,
    val sensorIdPrefix: String,
) {
    init {
        require(chainId.isNotBlank()) { "chainId must not be blank" }
        require(owner.isNotBlank()) { "owner must not be blank" }
        require(sensorIdPrefix.isNotBlank()) { "sensorIdPrefix must not be blank" }
        require(nodeUrl.isNotBlank()) { "nodeUrl must not be blank" }
        val uri = runCatching { URI(nodeUrl) }.getOrNull()
        require(uri != null && uri.scheme in setOf("http", "https") && !uri.host.isNullOrBlank()) {
            "nodeUrl must be an absolute HTTP(S) URL"
        }
    }

    fun sensorId(channelId: String): String {
        require(channelId.isNotBlank()) { "channelId must not be blank" }
        return "$sensorIdPrefix-$channelId"
    }
}
