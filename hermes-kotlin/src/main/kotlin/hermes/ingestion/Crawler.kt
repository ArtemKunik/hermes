package hermes.ingestion

import java.io.File
import java.nio.file.Path

private val SUPPORTED_EXTENSIONS = setOf(
    "rs", "tsx", "ts", "jsx", "js", "md", "toml", "json", "css",
    "kt", "kts", "java", "py", "go", "yaml", "yml"
)

private val IGNORED_DIRS = setOf(
    "target", "node_modules", ".git", ".venv", ".mypy_cache",
    ".pytest_cache", ".ruff_cache", "dist", ".next", ".vite",
    "build", ".gradle", ".idea", "out"
)

fun crawlDirectory(dir: Path): List<Path> {
    val files = mutableListOf<Path>()
    crawlRecursive(dir.toFile(), files)
    files.sort()
    return files
}

private fun crawlRecursive(dir: File, files: MutableList<Path>) {
    if (!dir.isDirectory) return
    if (IGNORED_DIRS.contains(dir.name)) return

    dir.listFiles()?.forEach { entry ->
        if (entry.isDirectory) {
            crawlRecursive(entry, files)
        } else if (isSupportedFile(entry)) {
            files.add(entry.toPath())
        }
    }
}

private fun isSupportedFile(file: File): Boolean {
    val ext = file.extension
    return SUPPORTED_EXTENSIONS.contains(ext)
}
