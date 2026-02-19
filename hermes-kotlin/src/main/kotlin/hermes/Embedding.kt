package hermes

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

private const val DEFAULT_MODEL = "text-embedding-004"
private const val DEFAULT_DIMENSION = 768
private const val DEFAULT_RPM = 60

class EmbeddingGenerator(
    private val apiKey: String = System.getenv("GEMINI_API_KEY")
        ?: throw IllegalStateException("GEMINI_API_KEY environment variable not set"),
    private val model: String = System.getenv("GEMINI_EMBEDDING_MODEL") ?: DEFAULT_MODEL,
    private val client: OkHttpClient = OkHttpClient()
) {
    companion object {
        fun dimension(): Int = DEFAULT_DIMENSION
    }

    @Serializable
    private data class EmbeddingRequest(
        val model: String,
        val content: EmbeddingContent
    )

    @Serializable
    private data class EmbeddingContent(val parts: List<EmbeddingPart>)

    @Serializable
    private data class EmbeddingPart(val text: String)

    @Serializable
    private data class EmbeddingResponse(val embedding: EmbeddingValues)

    @Serializable
    private data class EmbeddingValues(val values: List<Float>)

    private val json = Json { ignoreUnknownKeys = true }

    fun generateEmbedding(text: String): List<Float> {
        val url = "https://generativelanguage.googleapis.com/v1beta/models/$model:embedContent?key=$apiKey"

        val requestBody = json.encodeToString(
            EmbeddingRequest.serializer(),
            EmbeddingRequest(
                model = "models/$model",
                content = EmbeddingContent(parts = listOf(EmbeddingPart(text = text)))
            )
        )

        val request = Request.Builder()
            .url(url)
            .post(requestBody.toRequestBody("application/json".toMediaType()))
            .build()

        val response = client.newCall(request).execute()
        val body = response.body?.string() ?: throw RuntimeException("Empty response from embedding API")

        if (!response.isSuccessful) {
            throw RuntimeException("Embedding API returned ${response.code}: $body")
        }

        val parsed = json.decodeFromString(EmbeddingResponse.serializer(), body)
        return parsed.embedding.values
    }

    fun generateEmbeddings(texts: List<String>): List<List<Float>> {
        return texts.map { generateEmbedding(it) }
    }
}
