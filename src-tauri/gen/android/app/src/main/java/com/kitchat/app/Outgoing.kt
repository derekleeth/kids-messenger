package com.kitchat.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.ExifInterface
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.webkit.MimeTypeMap
import app.tauri.plugin.JSObject
import java.io.File
import java.util.UUID
import kotlin.math.max

/**
 * Gets a photo or video picked in (or shared to) KitChat ready to send: copies it into `cache/outgoing/` and makes
 * the small JPEG the chat shows for it (the photo at most 1920 pixels across, or a still from the video). The Rust
 * side (`send_media` in src-tauri/src/commands.rs) takes the location out of the full file and sends both.
 */
object Outgoing {
  private const val PREVIEW_SIDE = 1920
  private const val PREVIEW_QUALITY = 86
  const val MAX_BYTES = 1024L * 1024 * 1024

  class TooBig : Exception("That's too big to send (the most is 1 GB).")

  /** `{ kind, path, previewPath, width, height, durationMs, mime }`, or null for something that isn't a photo or video. */
  fun prepare(context: Context, uri: Uri, typeHint: String?): JSObject? {
    val mime = context.contentResolver.getType(uri) ?: typeHint ?: ""
    val video = mime.startsWith("video/")
    if (!video && !mime.startsWith("image/")) return null
    val dir = File(context.cacheDir, "outgoing").apply { mkdirs() }
    val ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: if (video) "mp4" else "jpg"
    val file = File(dir, "${UUID.randomUUID()}.$ext")
    copy(context, uri, file)
    val preview = File(dir, "${UUID.randomUUID()}.jpg")
    val item = JSObject()
    try {
      if (video) {
        val (w, h, duration) = videoPreview(file, preview)
        item.put("width", w)
        item.put("height", h)
        item.put("durationMs", duration)
      } else {
        val (w, h) = photoPreview(file, preview)
        item.put("width", w)
        item.put("height", h)
      }
    } catch (e: Exception) {
      if (!video) {
        file.delete()
        throw e
      }
      // A video Android can't take a still from still sends, with a plain card.
    }
    item.put("kind", if (video) "video" else "image")
    item.put("path", file.absolutePath)
    if (preview.exists()) item.put("previewPath", preview.absolutePath)
    item.put("mime", mime)
    return item
  }

  private fun copy(context: Context, uri: Uri, to: File) {
    var copied = 0L
    val input = context.contentResolver.openInputStream(uri) ?: throw IllegalStateException("can't open it")
    input.use {
      to.outputStream().use { output ->
        val buf = ByteArray(256 * 1024)
        while (true) {
          val n = input.read(buf)
          if (n < 0) break
          copied += n
          if (copied > MAX_BYTES) {
            output.close()
            to.delete()
            throw TooBig()
          }
          output.write(buf, 0, n)
        }
      }
    }
  }

  /** The shrunk photo, the right way up. Returns its full size as shown (rotation applied). */
  private fun photoPreview(file: File, preview: File): Pair<Int, Int> {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    val (w, h) = bounds.outWidth to bounds.outHeight
    if (w <= 0 || h <= 0) throw IllegalStateException("That picture can't be opened.")
    var sample = 1
    while (max(w, h) / (sample * 2) >= PREVIEW_SIDE) sample *= 2
    val decoded = BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })
      ?: throw IllegalStateException("That picture can't be opened.")
    val orientation = try {
      ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
    } catch (e: Exception) {
      ExifInterface.ORIENTATION_NORMAL
    }
    val scale = PREVIEW_SIDE.toFloat() / max(decoded.width, decoded.height)
    val matrix = Matrix()
    if (scale < 1f) matrix.postScale(scale, scale)
    when (orientation) {
      ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.postScale(-1f, 1f)
      ExifInterface.ORIENTATION_ROTATE_180 -> matrix.postRotate(180f)
      ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.postScale(1f, -1f)
      ExifInterface.ORIENTATION_TRANSPOSE -> { matrix.postRotate(90f); matrix.postScale(-1f, 1f) }
      ExifInterface.ORIENTATION_ROTATE_90 -> matrix.postRotate(90f)
      ExifInterface.ORIENTATION_TRANSVERSE -> { matrix.postRotate(-90f); matrix.postScale(-1f, 1f) }
      ExifInterface.ORIENTATION_ROTATE_270 -> matrix.postRotate(270f)
    }
    val shown = Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, matrix, true)
    save(shown, preview)
    if (shown !== decoded) decoded.recycle()
    shown.recycle()
    val turned = orientation in listOf(
      ExifInterface.ORIENTATION_TRANSPOSE, ExifInterface.ORIENTATION_ROTATE_90,
      ExifInterface.ORIENTATION_TRANSVERSE, ExifInterface.ORIENTATION_ROTATE_270,
    )
    return if (turned) h to w else w to h
  }

  private data class VideoInfo(val width: Int, val height: Int, val durationMs: Long)

  /** A still from near the start. Returns the video's size as shown, and its length. */
  private fun videoPreview(file: File, preview: File): VideoInfo {
    val retriever = MediaMetadataRetriever()
    try {
      retriever.setDataSource(file.path)
      fun num(key: Int) = retriever.extractMetadata(key)?.toLongOrNull() ?: 0L
      val duration = num(MediaMetadataRetriever.METADATA_KEY_DURATION)
      val rotation = num(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)
      var w = num(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH).toInt()
      var h = num(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT).toInt()
      if (rotation == 90L || rotation == 270L) w = h.also { h = w }
      val at = minOf(500L, duration / 2) * 1000
      val frame = retriever.getScaledFrameAtTime(at, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, PREVIEW_SIDE, PREVIEW_SIDE)
      if (frame != null) {
        if (w <= 0 || h <= 0) { w = frame.width; h = frame.height }
        save(frame, preview)
        frame.recycle()
      }
      return VideoInfo(w, h, duration)
    } finally {
      retriever.release()
    }
  }

  private fun save(bitmap: Bitmap, to: File) {
    // A Pixel's Ultra HDR photos carry a gain map, which compress() would write too: the chat's picture is plain.
    if (Build.VERSION.SDK_INT >= 34) bitmap.gainmap = null
    to.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, PREVIEW_QUALITY, it) }
  }

  /** Throws away whatever is left in `cache/outgoing/` from earlier (a send that was never finished). */
  fun clearOld(context: Context) {
    val cutoff = System.currentTimeMillis() - 24L * 60 * 60 * 1000
    File(context.cacheDir, "outgoing").listFiles()?.forEach { if (it.lastModified() < cutoff) it.delete() }
  }
}
