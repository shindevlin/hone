package network.hone.app.sound

import android.content.Context
import org.json.JSONObject
import org.tensorflow.lite.Interpreter
import java.io.Closeable
import java.io.FileInputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.channels.FileChannel

/**
 * Perch v2 (Google DeepMind, Apache-2.0), the 8-bit LiteRT build published by Arm
 * (huggingface.co/Arm/perch-v2-int8-litert). Input: 5 s of mono audio at 32 kHz as float.
 * Output used here: softmax over 14,795 classes (species plus general sound classes).
 * The model and labels are fetched into assets by tools/fetch-perch.sh (hash-checked).
 */
class Perch(ctx: Context) : Closeable {
    private val interp: Interpreter
    val names: Array<String>
    private val labelOut: Int
    private val input = ByteBuffer.allocateDirect(4 * FRAME).order(ByteOrder.nativeOrder())
    private val output: ByteBuffer

    /** Classes whose presence means people are audible. A frame with any of them is dropped. */
    val humanIdx: IntArray
    /** Species classes (binomial names), excluding Homo sapiens. */
    val isSpecies: BooleanArray

    init {
        val fd = ctx.assets.openFd("perch/$MODEL_FILE")
        val buf = FileInputStream(fd.fileDescriptor).channel
            .map(FileChannel.MapMode.READ_ONLY, fd.startOffset, fd.declaredLength)
        interp = Interpreter(buf, Interpreter.Options().setNumThreads(4))

        val j = JSONObject(ctx.assets.open("perch/class_names.json").bufferedReader().readText())
        val n = j.length()
        names = Array(n) { "" }
        j.keys().forEach { k -> names[j.getInt(k) - 1] = k }
        labelOut = (0 until interp.outputTensorCount).first { interp.getOutputTensor(it).shape().last() == n }
        output = ByteBuffer.allocateDirect(4 * n).order(ByteOrder.nativeOrder())
        humanIdx = names.indices.filter { HUMAN.containsMatchIn(names[it]) }.toIntArray()
        isSpecies = BooleanArray(n) { names[it].contains(' ') && names[it] != "Homo sapiens" }
    }

    /** Returns class probabilities for one 5 s frame (exactly [FRAME] samples). */
    fun classify(audio: FloatArray): FloatArray {
        require(audio.size == FRAME)
        input.clear(); input.asFloatBuffer().put(audio)
        output.clear()
        val outs = HashMap<Int, Any>().apply { put(labelOut, output) }
        interp.runForMultipleInputsOutputs(arrayOf<Any>(input), outs)
        output.rewind()
        val p = FloatArray(names.size); output.asFloatBuffer().get(p)
        // Don't keep audio around longer than needed.
        input.clear(); repeat(FRAME) { input.putFloat(0f) }
        return p
    }

    override fun close() = interp.close()

    companion object {
        const val SAMPLE_RATE = 32_000
        const val FRAME = 160_000
        const val MODEL_FILE = "perch_v2_8bit.tflite"
        const val MODEL_ID = "perch-v2-int8-litert@arm"
        const val MODEL_SHA256 = "9ccbfc424bf35efd1d1f4604a3339c2bb9e03b1c7f2ca7b8ccea7f0a026ceb0c"

        // Speech, singing, laughter, crowds and the Homo sapiens class. Conservative on purpose:
        // dropping a frame with a person in it costs one bird frame; keeping it is not allowed.
        private val HUMAN = Regex(
            "(?i)speech|speaking|voice|conversation|whisper|singing|shout|scream|laugh|giggle|" +
                "chuckle|chatter|crowd|crying|babbl|cough|sneeze|^homo sapiens$|human"
        )
    }
}
