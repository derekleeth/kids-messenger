//! The device token on Android, sealed with an AES-256-GCM key that lives in the Android Keystore (StrongBox where the
//! device has one, otherwise the TEE) and never leaves it. The sealed token is `device-token.keystore` in the app's
//! private folder (Auto Backup is off), so a copy of that folder is useless without the tablet itself.
//!
//! Adapted from TerminalLynx's `security/android_keystore.rs`: the Keystore is reached through JNI on the calling
//! thread, using only framework classes, so it works from any thread with no plugin round-trip.

use std::path::{Path, PathBuf};

use jni::objects::{JByteArray, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const KEY_ALIAS: &str = "kitchat.device-token.v1";
const KEYSTORE: &str = "AndroidKeyStore";
const TRANSFORMATION: &str = "AES/GCM/NoPadding";
const GCM_TAG_BITS: i32 = 128;
const PURPOSES: i32 = 1 | 2;
const ENCRYPT_MODE: i32 = 1;
const DECRYPT_MODE: i32 = 2;
const AAD: &[u8] = b"kitchat device token";

#[derive(Serialize, Deserialize)]
struct Sealed {
    version: u32,
    iv: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("device-token.keystore")
}

pub fn save(dir: &Path, token: &str) -> AppResult<()> {
    ensure_key()?;
    let (iv, ciphertext) = with_env(|env| encrypt(env, token.as_bytes()))?;
    let sealed = Sealed { version: 1, iv, ciphertext };
    let path = path(dir);
    let tmp = path.with_extension("keystore.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&sealed)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load(dir: &Path) -> AppResult<Option<String>> {
    let raw = match std::fs::read(path(dir)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let sealed: Sealed = serde_json::from_slice(&raw)?;
    let plain = with_env(|env| decrypt(env, &sealed.iv, &sealed.ciphertext))?;
    // The Keystore key is gone (app data restored on another tablet): the device has to join again.
    Ok(plain.and_then(|p| String::from_utf8(p).ok()))
}

pub fn delete(dir: &Path) {
    let _ = std::fs::remove_file(path(dir));
}

fn with_env<T>(f: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>) -> AppResult<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process's JavaVM pointer, set by the Activity before Rust runs.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| AppError::Other(e.to_string()))?;
    let mut env = vm.attach_current_thread().map_err(|e| AppError::Other(e.to_string()))?;
    match env.with_local_frame(32, |env| f(env)) {
        Ok(value) => Ok(value),
        Err(e) => {
            let _ = env.exception_clear();
            Err(AppError::Other(format!("Android Keystore: {e}")))
        }
    }
}

fn ensure_key() -> AppResult<()> {
    if with_env(|env| Ok(!secret_key(env)?.is_null()))? {
        return Ok(());
    }
    if let Err(e) = with_env(|env| generate_key(env, true)) {
        log::info!("no StrongBox for the device token ({e}); using the TEE");
        with_env(|env| generate_key(env, false))?;
    }
    Ok(())
}

fn secret_key<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JObject<'a>> {
    let name = env.new_string(KEYSTORE)?;
    let store = env
        .call_static_method(
            "java/security/KeyStore",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    env.call_method(
        &store,
        "load",
        "(Ljava/security/KeyStore$LoadStoreParameter;)V",
        &[JValue::Object(&JObject::null())],
    )?;
    let alias = env.new_string(KEY_ALIAS)?;
    env.call_method(
        &store,
        "getKey",
        "(Ljava/lang/String;[C)Ljava/security/Key;",
        &[JValue::Object(&alias), JValue::Object(&JObject::null())],
    )?
    .l()
}

fn generate_key(env: &mut JNIEnv, strongbox: bool) -> jni::errors::Result<()> {
    const BUILDER: &str = "android/security/keystore/KeyGenParameterSpec$Builder";
    const RETURNS_BUILDER: &str = "Landroid/security/keystore/KeyGenParameterSpec$Builder;";
    let alias = env.new_string(KEY_ALIAS)?;
    let builder =
        env.new_object(BUILDER, "(Ljava/lang/String;I)V", &[JValue::Object(&alias), JValue::Int(PURPOSES)])?;
    let gcm = env.new_string("GCM")?;
    let modes = env.new_object_array(1, "java/lang/String", &gcm)?;
    env.call_method(
        &builder,
        "setBlockModes",
        format!("([Ljava/lang/String;){RETURNS_BUILDER}"),
        &[JValue::Object(&modes)],
    )?;
    let no_padding = env.new_string("NoPadding")?;
    let paddings = env.new_object_array(1, "java/lang/String", &no_padding)?;
    env.call_method(
        &builder,
        "setEncryptionPaddings",
        format!("([Ljava/lang/String;){RETURNS_BUILDER}"),
        &[JValue::Object(&paddings)],
    )?;
    env.call_method(&builder, "setKeySize", format!("(I){RETURNS_BUILDER}"), &[JValue::Int(256)])?;
    if strongbox {
        env.call_method(&builder, "setIsStrongBoxBacked", format!("(Z){RETURNS_BUILDER}"), &[JValue::Bool(1)])?;
    }
    let spec = env.call_method(&builder, "build", "()Landroid/security/keystore/KeyGenParameterSpec;", &[])?.l()?;
    let aes = env.new_string("AES")?;
    let provider = env.new_string(KEYSTORE)?;
    let generator = env
        .call_static_method(
            "javax/crypto/KeyGenerator",
            "getInstance",
            "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
            &[JValue::Object(&aes), JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(&generator, "init", "(Ljava/security/spec/AlgorithmParameterSpec;)V", &[JValue::Object(&spec)])?;
    env.call_method(&generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?;
    Ok(())
}

fn cipher<'a>(env: &mut JNIEnv<'a>) -> jni::errors::Result<JObject<'a>> {
    let transformation = env.new_string(TRANSFORMATION)?;
    env.call_static_method(
        "javax/crypto/Cipher",
        "getInstance",
        "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
        &[JValue::Object(&transformation)],
    )?
    .l()
}

fn encrypt(env: &mut JNIEnv, plain: &[u8]) -> jni::errors::Result<(Vec<u8>, Vec<u8>)> {
    let key = secret_key(env)?;
    let cipher = cipher(env)?;
    env.call_method(&cipher, "init", "(ILjava/security/Key;)V", &[JValue::Int(ENCRYPT_MODE), JValue::Object(&key)])?;
    let aad = env.byte_array_from_slice(AAD)?;
    env.call_method(&cipher, "updateAAD", "([B)V", &[JValue::Object(&aad)])?;
    let input = env.byte_array_from_slice(plain)?;
    let sealed = JByteArray::from(env.call_method(&cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?.l()?);
    let iv = JByteArray::from(env.call_method(&cipher, "getIV", "()[B", &[])?.l()?);
    Ok((env.convert_byte_array(&iv)?, env.convert_byte_array(&sealed)?))
}

fn decrypt(env: &mut JNIEnv, iv: &[u8], sealed: &[u8]) -> jni::errors::Result<Option<Vec<u8>>> {
    let key = secret_key(env)?;
    if key.is_null() {
        return Ok(None);
    }
    let cipher = cipher(env)?;
    let iv = env.byte_array_from_slice(iv)?;
    let spec = env.new_object(
        "javax/crypto/spec/GCMParameterSpec",
        "(I[B)V",
        &[JValue::Int(GCM_TAG_BITS), JValue::Object(&iv)],
    )?;
    env.call_method(
        &cipher,
        "init",
        "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[JValue::Int(DECRYPT_MODE), JValue::Object(&key), JValue::Object(&spec)],
    )?;
    let aad = env.byte_array_from_slice(AAD)?;
    env.call_method(&cipher, "updateAAD", "([B)V", &[JValue::Object(&aad)])?;
    let input = env.byte_array_from_slice(sealed)?;
    let plain = JByteArray::from(env.call_method(&cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?.l()?);
    Ok(Some(env.convert_byte_array(&plain)?))
}
