package network.hone.app.data

/**
 * Explicitly exposes both Shin-approved authorization paths. The owner signer is
 * supplied only for the operation that needs it and is never retained.
 */
class SensorSubmissionCoordinator(
    private val client: SensorCommitClient,
) {
    fun enrollDevice(
        sensorId: String,
        owner: String,
        device: DeviceSensorIdentity,
        ownerPostingSigner: SensorSigner,
    ): SensorHttpResponse = client.enrollDevice(
        sensorId = sensorId,
        owner = owner,
        devicePublicKeyHex = device.publicKeyHex,
        ownerPostingSigner = ownerPostingSigner,
    )

    fun submitWithEnrolledDevice(
        batch: SensorReadingBatch,
        device: DeviceSensorIdentity,
    ): SensorHttpResponse = client.postReading(batch, device.signer)

    fun submitWithOwnerFallback(
        batch: SensorReadingBatch,
        ownerPostingSigner: SensorSigner,
    ): SensorHttpResponse = client.postReading(batch, ownerPostingSigner)

    fun revokeDevice(
        sensorId: String,
        owner: String,
        ownerPostingSigner: SensorSigner,
    ): SensorHttpResponse = client.revokeDevice(sensorId, owner, ownerPostingSigner)
}
