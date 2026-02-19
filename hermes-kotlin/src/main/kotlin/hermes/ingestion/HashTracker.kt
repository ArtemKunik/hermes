package hermes.ingestion

import java.io.File
import java.nio.file.Path
import java.security.MessageDigest
import java.sql.Connection

class HashTracker(
    private val db: Connection,
    private val projectId: String
) {
    fun isUnchanged(filePath: String): Boolean {
        val storedHash = db.prepareStatement(
            "SELECT content_hash FROM file_hashes WHERE file_path = ? AND project_id = ?"
        ).use { ps ->
            ps.setString(1, filePath)
            ps.setString(2, projectId)
            ps.executeQuery().use { rs ->
                if (rs.next()) rs.getString(1) else null
            }
        } ?: return false

        val content = runCatching { File(filePath).readText() }.getOrNull() ?: return false
        val currentHash = computeHash(content)
        return storedHash == currentHash
    }

    fun updateHash(filePath: String, actualPath: Path) {
        val content = actualPath.toFile().readText()
        val hash = computeHash(content)
        db.prepareStatement(
            """INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
               VALUES (?, ?, ?, datetime('now'))"""
        ).use { ps ->
            ps.setString(1, filePath)
            ps.setString(2, projectId)
            ps.setString(3, hash)
            ps.executeUpdate()
        }
    }

    fun isChunkUnchanged(chunkKey: String, currentHash: String): Boolean {
        val stored = db.prepareStatement(
            "SELECT content_hash FROM file_hashes WHERE file_path = ? AND project_id = ?"
        ).use { ps ->
            ps.setString(1, chunkKey)
            ps.setString(2, projectId)
            ps.executeQuery().use { rs ->
                if (rs.next()) rs.getString(1) else null
            }
        }
        return stored == currentHash
    }

    fun updateChunkHash(chunkKey: String, hash: String) {
        db.prepareStatement(
            """INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
               VALUES (?, ?, ?, datetime('now'))"""
        ).use { ps ->
            ps.setString(1, chunkKey)
            ps.setString(2, projectId)
            ps.setString(3, hash)
            ps.executeUpdate()
        }
    }
}

fun computeHash(content: String): String {
    val digest = MessageDigest.getInstance("SHA-256")
    val hashBytes = digest.digest(content.toByteArray())
    return hashBytes.joinToString("") { "%02x".format(it) }
}
