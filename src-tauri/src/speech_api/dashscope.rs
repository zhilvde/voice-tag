use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use super::{
    curl_command, error_detail, json, parse_transcript, post_json, request, send, ApiSegment,
    ApiTranscript, Project, Result, Route, SpeechWave, Transcript, Url, Uuid, Value,
};

pub(super) fn transcribe(
    endpoint: &Url,
    project: &Project,
    model: &str,
    route: Route,
    language: &str,
    key: &str,
    progress: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Transcript> {
    progress("正在准备百炼临时上传")?;
    let source = Path::new(&project.audio_path);
    let diarization = route == Route::Paraformer;
    let converted = match source.extension().and_then(|ext| ext.to_str()) {
        Some("aac" | "amr" | "flac" | "m4a" | "mp3" | "ogg" | "wav") if !diarization => None,
        _ => Some(SpeechWave::prepare(source)?),
    };
    let path = converted.as_ref().map_or(source, SpeechWave::path);
    let file_url = upload(endpoint, model, key, path, progress)?;

    progress("正在提交百炼异步转写任务")?;
    let (input, parameters) = if diarization {
        let mut parameters = json!({"channel_id": [0], "diarization_enabled": true});
        if language != "auto" {
            parameters["language_hints"] = json!([language]);
        }
        (json!({"file_urls": [file_url]}), parameters)
    } else {
        let mut parameters = json!({"channel_id": [0], "enable_itn": false, "enable_words": true});
        if language != "auto" {
            parameters["language"] = json!(language);
        }
        (json!({"file_url": file_url}), parameters)
    };
    let submitted = post_json(
        endpoint,
        key,
        json!({"model": model, "input": input, "parameters": parameters}),
        &[
            "X-DashScope-Async: enable",
            "X-DashScope-OssResourceResolve: enable",
        ],
    )
    .map_err(|error| format!("百炼任务提交失败。{error}"))?;
    let task_id = submitted["output"]["task_id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .ok_or_else(|| {
            format!(
                "百炼未返回有效任务编号。{}",
                error_detail(&submitted, Some(key))
            )
        })?;
    let result_url = wait_for_result(endpoint, key, task_id, route, progress)?;
    progress("正在读取百炼转写结果")?;
    let result: Value = serde_json::from_str(&send(curl_command(), &result_url, None)?)
        .map_err(|_| "百炼转写结果不是有效的 JSON。")?;
    let transcripts = result["transcripts"]
        .as_array()
        .ok_or("百炼未返回转写片段。")?;
    let mut segments: Vec<_> = transcripts
        .iter()
        .filter_map(|entry| entry["sentences"].as_array())
        .flatten()
        .filter_map(|sentence| {
            Some(ApiSegment {
                text: sentence["text"].as_str()?.to_owned(),
                start: sentence["begin_time"].as_f64()? / 1_000.0,
                end: sentence["end_time"].as_f64()? / 1_000.0,
                speaker: sentence["speaker_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        sentence["speaker_id"]
                            .as_i64()
                            .map(|speaker| speaker.to_string())
                    }),
            })
        })
        .collect();
    segments.sort_by(|left, right| left.start.total_cmp(&right.start));
    let text = transcripts
        .iter()
        .filter_map(|entry| entry["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    parse_transcript(ApiTranscript { text, segments }, project.duration)
}

fn service_url(endpoint: &Url, path: &str) -> Url {
    let mut url = endpoint.clone();
    url.set_path(path);
    url.set_query(None);
    url
}

fn temporary_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).map_err(|_| "百炼返回的临时下载或上传地址无效。")?;
    if cfg!(test) && url.host_str() == Some("127.0.0.1") && url.scheme() == "http" {
        return Ok(url);
    }
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with(".aliyuncs.com"))
    {
        return Err("百炼返回的临时地址不属于阿里云。".into());
    }
    // Some signed OSS result links are returned with an HTTP scheme.
    url.set_scheme("https")
        .map_err(|_| "无法使用 HTTPS 访问百炼临时地址。")?;
    Ok(url)
}

fn upload(
    endpoint: &Url,
    model: &str,
    key: &str,
    path: &Path,
    progress: &mut impl FnMut(&str) -> Result<()>,
) -> Result<String> {
    let mut policy_url = service_url(endpoint, "/api/v1/uploads");
    let regional_host = match endpoint.host_str().unwrap_or_default() {
        host if host.ends_with(".cn-beijing.maas.aliyuncs.com") => Some("dashscope.aliyuncs.com"),
        host if host.ends_with(".ap-southeast-1.maas.aliyuncs.com") => {
            Some("dashscope-intl.aliyuncs.com")
        }
        _ => None,
    };
    if let Some(host) = regional_host {
        policy_url
            .set_host(Some(host))
            .map_err(|_| "百炼临时上传区域无效。")?;
    }
    policy_url
        .query_pairs_mut()
        .append_pair("action", "getPolicy")
        .append_pair("model", model);
    let mut command = curl_command();
    command.args(["--header", "Content-Type: application/json"]);
    let response = request(command, &policy_url, key)
        .map_err(|error| format!("获取百炼临时上传凭证失败。{error}"))?;
    let response: Value = serde_json::from_str(&response).map_err(|_| "百炼上传凭证无法解析。")?;
    let policy = &response["data"];
    let value = |name: &str| {
        policy[name]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "百炼上传凭证缺少 {name}。{}",
                    error_detail(&response, Some(key))
                )
            })
    };
    let host = temporary_url(value("upload_host")?)?;
    let acl = value("x_oss_object_acl")?;
    if acl != "private" {
        return Err("百炼未提供私有上传权限。".into());
    }
    let max_size = &policy["max_file_size_mb"];
    if let Some(limit) = max_size
        .as_f64()
        .or_else(|| max_size.as_str().and_then(|size| size.parse().ok()))
    {
        let bytes = fs::metadata(path)
            .map_err(|error| format!("无法读取音频大小：{error}"))?
            .len();
        if limit > 0.0 && bytes as f64 > limit * 1_000_000.0 {
            return Err(format!(
                "录音超过百炼临时存储限额（{limit} MB）。可使用较小的 MP3 或内置离线识别。"
            ));
        }
    }
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("wav");
    let filename = format!("voicetag-{}.{ext}", Uuid::new_v4());
    let object = format!(
        "{}/{}",
        value("upload_dir")?.trim_end_matches('/'),
        filename
    );
    let mut command = curl_command();
    for (field, value) in [
        ("OSSAccessKeyId", value("oss_access_key_id")?),
        ("Signature", value("signature")?),
        ("policy", value("policy")?),
        ("x-oss-object-acl", acl),
        ("x-oss-forbid-overwrite", value("x_oss_forbid_overwrite")?),
        ("key", &object),
        ("success_action_status", "200"),
    ] {
        command.args(["--form-string", &format!("{field}={value}")]);
    }
    let path = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    command.args(["--form", &format!("file=@\"{path}\";filename={filename}")]);
    progress("正在将录音上传到百炼私有临时存储")?;
    send(command, &host, None).map_err(|error| format!("百炼临时录音上传失败。{error}"))?;
    Ok(format!("oss://{object}"))
}

fn wait_for_result(
    endpoint: &Url,
    key: &str,
    task_id: &str,
    route: Route,
    progress: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Url> {
    let url = service_url(endpoint, &format!("/api/v1/tasks/{task_id}"));
    let deadline = Instant::now() + Duration::from_secs(1_800);
    let mut previous = "";
    loop {
        let mut command = curl_command();
        command.args(["--max-time", "30"]);
        let body = request(command, &url, key)
            .map_err(|error| format!("无法查询百炼任务 {task_id}。{error}"))?;
        let response: Value = serde_json::from_str(&body).map_err(|_| "百炼任务状态无法解析。")?;
        let output = &response["output"];
        let stage = match output["task_status"].as_str() {
            Some("SUCCEEDED") => {
                let result = if route == Route::Paraformer {
                    let result = &output["results"][0];
                    if result["subtask_status"].as_str() != Some("SUCCEEDED") {
                        let detail = error_detail(result, Some(key));
                        return Err(format!("百炼任务 {task_id} 的录音转写失败。{detail}"));
                    }
                    result
                } else {
                    &output["result"]
                };
                let url = result["transcription_url"]
                    .as_str()
                    .ok_or("百炼任务缺少结果下载地址。")?;
                return temporary_url(url);
            }
            Some("PENDING") => "百炼转写任务排队中",
            Some("RUNNING") => "百炼正在转写录音",
            _ => {
                let details = error_detail(output, Some(key));
                let details = if details.is_empty() {
                    error_detail(&response, Some(key))
                } else {
                    details
                };
                return Err(format!("百炼任务 {task_id} 未完成。{details}"));
            }
        };
        if previous != stage {
            progress(stage)?;
            previous = stage;
        }
        if Instant::now() >= deadline {
            return Err(format!("等待百炼任务超时。可在百炼中查询任务 {task_id}。"));
        }
        if !cfg!(test) {
            thread::sleep(Duration::from_secs(3));
        }
    }
}
