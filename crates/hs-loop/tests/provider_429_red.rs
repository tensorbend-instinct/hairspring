//! A spend-ceiling 429 must read as a clear, non-retryable message, and an
//! HTML error body must not leak markup into the model's feedback.
use hs_loop::realmodel::provider_error_message;

#[test]
fn spend_ceiling_429_is_clear_and_says_retry_is_futile() {
    let html = "<!DOCTYPE HTML><html><body><h1>Error response</h1><p>Error code: 429</p><p>Message: mission spend ceiling.</p></body></html>";
    let m = provider_error_message("deepseek", 429, html);
    assert!(m.contains("spend ceiling"), "{m}");
    assert!(m.contains("retrying will not help"), "{m}");
    assert!(!m.contains('<'), "markup leaked: {m}");
}

#[test]
fn ordinary_429_and_html_bodies_are_plain_text() {
    let m = provider_error_message("deepseek", 429, "<html><p>Too many requests</p></html>");
    assert!(m.contains("Too many requests") && !m.contains('<'), "{m}");
    assert!(!m.contains("retrying will not help"), "{m}");
    assert_eq!(provider_error_message("x", 500, ""), "x: HTTP 500 from provider");
}

#[test]
fn ceiling_429_message_is_not_retryable_but_plain_429_is() {
    use hs_loop::realmodel::is_retryable_status;
    let ceiling = provider_error_message("deepseek", 429, "Message: mission spend ceiling.");
    assert!(!is_retryable_status(429, &ceiling));
    let plain = provider_error_message("deepseek", 429, "slow down");
    assert!(is_retryable_status(429, &plain));
    assert!(is_retryable_status(503, ""));
    assert!(!is_retryable_status(400, ""));
}
