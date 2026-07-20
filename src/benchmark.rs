use crate::models::*;
use crate::state::AppState;
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Instant;

const MAX_SAVED_BODY_BYTES: usize = 1024 * 1024;

fn truncate_body(body: String) -> (String, bool) {
    if body.len() <= MAX_SAVED_BODY_BYTES { return (body, false); }
    let mut end = MAX_SAVED_BODY_BYTES;
    while !body.is_char_boundary(end) { end -= 1; }
    (body[..end].to_string(), true)
}

pub(crate) fn parse_judge_response(raw: &str) -> Result<(f64, f64, f64, f64, f64, String, f64), String> {
    let envelope: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let value: Value = envelope["choices"].get(0).and_then(|choice| choice["message"]["content"].as_str())
        .and_then(|content| serde_json::from_str(content).ok()).unwrap_or(envelope);
    let number = |key: &str| value.get(key).and_then(Value::as_f64).filter(|v| (0.0..=100.0).contains(v)).ok_or_else(|| format!("{} 必须在 0 到 100 之间", key));
    let confidence = value.get("confidence").and_then(Value::as_f64).filter(|v| (0.0..=1.0).contains(v)).ok_or_else(|| "confidence 必须在 0 到 1 之间".to_string())?;
    Ok((number("score")?, number("accuracy")?, number("completeness")?, number("instruction_following")?, number("writing_quality")?, value.get("reason").and_then(Value::as_str).unwrap_or_default().to_string(), confidence))
}

fn extract_streamed_text(raw: &str) -> Option<String> {
    let mut output = String::new();
    for line in raw.lines().filter_map(|line| line.strip_prefix("data:").map(str::trim)) {
        if line == "[DONE]" { continue; }
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue; };
        let openai = event["choices"].as_array().map(|choices| choices.iter().filter_map(|choice| choice["delta"]["content"].as_str()).collect::<String>()).unwrap_or_default();
        let responses = event["delta"].as_str().filter(|_| event["type"] == "response.output_text.delta").unwrap_or_default();
        let anthropic = event["delta"]["text"].as_str().filter(|_| event["type"] == "content_block_delta").unwrap_or_default();
        output.push_str(&openai);
        output.push_str(responses);
        output.push_str(anthropic);
    }
    (!output.is_empty()).then_some(output)
}

fn usage_total(value: &Value) -> Option<u64> {
    let usage = value.get("usage")
        .or_else(|| value.get("message").and_then(|message| message.get("usage")))
        .or_else(|| value.get("response").and_then(|response| response.get("usage")))?;
    usage.get("total_tokens").and_then(Value::as_u64)
        .or_else(|| usage.get("input_tokens").and_then(Value::as_u64).zip(usage.get("output_tokens").and_then(Value::as_u64)).map(|(input, output)| input + output))
        .or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64).zip(usage.get("completion_tokens").and_then(Value::as_u64)).map(|(input, output)| input + output))
}

fn response_tokens(raw: &str) -> Option<u64> {
    if let Ok(response) = serde_json::from_str::<Value>(raw) {
        return usage_total(&response);
    }

    let mut total = None;
    let mut input = None;
    let mut output = None;
    for event in raw.lines().filter_map(|line| line.strip_prefix("data:").map(str::trim)).filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        total = usage_total(&event).or(total);
        if let Some(usage) = event.get("usage")
            .or_else(|| event.get("message").and_then(|message| message.get("usage")))
            .or_else(|| event.get("response").and_then(|response| response.get("usage"))) {
            input = usage.get("input_tokens").and_then(Value::as_u64).or(input).or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64));
            output = usage.get("output_tokens").and_then(Value::as_u64).or(output).or_else(|| usage.get("completion_tokens").and_then(Value::as_u64));
        }
    }
    total.or_else(|| input.zip(output).map(|(input, output)| input + output))
}

fn benchmark_url(endpoint: &EndpointConfig) -> String {
    if endpoint.api_type == ApiType::Custom { return endpoint.url.clone(); }
    let path = match endpoint.api_type { ApiType::Anthropic => "v1/messages", ApiType::OpenAIResponses => "v1/responses", _ => "v1/chat/completions" };
    let base = endpoint.url.trim_end_matches('/');
    if base.ends_with("/v1") { format!("{}/{}", base, path.trim_start_matches("v1/")) } else { format!("{}/{}", base, path) }
}

async fn call_endpoint(state: &AppState, endpoint: &EndpointConfig, body: Value) -> (String, Option<u16>, Option<u64>, u64, String, Option<u64>, Option<String>) {
    let started = Instant::now();
    let body = if endpoint.api_type == ApiType::OpenAI { body } else { crate::converter::convert_request(&body, &ApiType::OpenAI, &endpoint.api_type) };
    let mut builder = state.http_client.post(benchmark_url(endpoint)).json(&body).timeout(std::time::Duration::from_secs(endpoint.timeout));
    builder = match endpoint.api_type { ApiType::Anthropic => builder.header("x-api-key", &endpoint.api_key).header("anthropic-version", "2023-06-01"), _ => builder.bearer_auth(&endpoint.api_key) };
    let response = match builder.send().await { Ok(response) => response, Err(error) => return (if error.is_timeout() { "timeout" } else { "error" }.to_string(), None, None, started.elapsed().as_millis() as u64, String::new(), None, Some(error.to_string())) };
    let status_code = response.status().as_u16();
    let mut stream = response.bytes_stream();
    let mut raw = Vec::new();
    let mut ttft_ms = None;
    while let Some(chunk) = stream.next().await {
        match chunk { Ok(chunk) => { if ttft_ms.is_none() { ttft_ms = Some(started.elapsed().as_millis() as u64); } raw.extend_from_slice(&chunk); }, Err(error) => return ("stream_interrupted".to_string(), Some(status_code), ttft_ms, started.elapsed().as_millis() as u64, String::from_utf8_lossy(&raw).into_owned(), None, Some(error.to_string())) }
    }
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let output = extract_streamed_text(&raw).unwrap_or_else(|| raw.clone());
    let status = if (200..300).contains(&status_code) { "success" } else { "error" };
    let error = (status == "error").then_some(raw.clone());
    (status.to_string(), Some(status_code), ttft_ms, started.elapsed().as_millis() as u64, output, response_tokens(&raw), error)
}

pub async fn execute_benchmark_run(state: &AppState, run_id: &str) {
    let Some(mut run) = state.get_model_benchmark(run_id) else { return; };
    run.status = ModelBenchmarkStatus::Running;
    state.update_model_benchmark(run.clone());

    for case in run.cases.clone() {
        for target in run.benchmark_targets() {
            let Some(endpoint) = run.endpoint_snapshots.iter().find(|endpoint| endpoint.id == target.endpoint_id).cloned() else { continue; };
            for attempt_number in 1..=run.attempts_per_case {
                if matches!(state.get_model_benchmark(run_id).map(|r| r.status), Some(ModelBenchmarkStatus::Cancelled)) { return; }
                let mut request = json!({"model": target.model, "messages": case.messages.clone(), "stream": true});
                if matches!(endpoint.api_type, ApiType::OpenAI | ApiType::Custom) {
                    request["stream_options"] = json!({"include_usage": true});
                }
                let (status, status_code, ttft_ms, duration_ms, output, total_tokens, error_message) = call_endpoint(state, &endpoint, request).await;
                let (output, output_truncated) = truncate_body(output);
                let attempt_id = uuid::Uuid::new_v4().to_string();
                let success = status == "success";
                run.attempts.push(ModelBenchmarkAttempt { id: attempt_id.clone(), case_id: case.id.clone(), endpoint_id: endpoint.id.clone(), endpoint_name: endpoint.name.clone(), model: target.model.clone(), attempt_number, status, status_code, ttft_ms, duration_ms, total_tokens, output: output.clone(), output_truncated, error_message });
                if success {
                    let judge_endpoint = state.get_endpoint(&run.judge.endpoint_id).map(|endpoint| endpoint.config);
                    let judge_result = match judge_endpoint {
                        Some(judge_endpoint) => {
                            let prompt = format!("评测样本：{}\n候选输出：{}\n评分标准：{}\n返回 JSON，包含 score、accuracy、completeness、instruction_following、writing_quality、reason、confidence。所有评分为 0-100，confidence 为 0-1。", case.name, output, run.judge.rubric);
                            let request = json!({"model": run.judge.model, "messages": [{"role":"user","content":prompt}], "stream": false});
                            let (judge_status, _, _, _, raw, _, judge_error) = call_endpoint(state, &judge_endpoint, request).await;
                            let (raw_response, response_truncated) = truncate_body(raw.clone());
                            match parse_judge_response(&raw) {
                                Ok((score, accuracy, completeness, instruction_following, writing_quality, reason, confidence)) if judge_status == "success" => ModelBenchmarkJudgeResult { attempt_id, status: "success".to_string(), score: Some(score), accuracy: Some(accuracy), completeness: Some(completeness), instruction_following: Some(instruction_following), writing_quality: Some(writing_quality), reason: Some(reason), confidence: Some(confidence), raw_response, response_truncated },
                                _ => ModelBenchmarkJudgeResult { attempt_id, status: if judge_error.is_some() { "judge_failed".to_string() } else { "judge_parse_error".to_string() }, score: None, accuracy: None, completeness: None, instruction_following: None, writing_quality: None, reason: judge_error, confidence: None, raw_response, response_truncated },
                            }
                        }
                        None => ModelBenchmarkJudgeResult { attempt_id, status: "judge_failed".to_string(), score: None, accuracy: None, completeness: None, instruction_following: None, writing_quality: None, reason: Some("评审端点不存在".to_string()), confidence: None, raw_response: String::new(), response_truncated: false },
                    };
                    run.judge_results.push(judge_result);
                }
                state.update_model_benchmark(run.clone());
            }
        }
    }
    run.status = ModelBenchmarkStatus::Completed;
    run.completed_at = Some(Utc::now());
    state.update_model_benchmark(run);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_judge_response_validates_percentile_fields() {
        let raw = r#"{"score":90,"accuracy":91,"completeness":92,"instruction_following":93,"writing_quality":94,"reason":"ok","confidence":0.8}"#;
        assert_eq!(parse_judge_response(raw).unwrap().0, 90.0);
        assert!(parse_judge_response(r#"{"score":101}"#).is_err());
    }

    #[test]
    fn extract_streamed_text_joins_openai_deltas() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\ndata: [DONE]\n";
        assert_eq!(extract_streamed_text(raw), Some("Hello world".to_string()));
    }

    #[test]
    fn response_tokens_reads_openai_and_anthropic_stream_usage() {
        let openai = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"total_tokens\":12}}\n\ndata: [DONE]\n";
        let anthropic = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":7}}}\n\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n";
        assert_eq!(response_tokens(openai), Some(12));
        assert_eq!(response_tokens(anthropic), Some(12));
    }
}
