//! Retry limits must survive the actual request -> queue -> completion path.
//! These tests never call record_failure directly or wait on a real clock.
use yu_assets::{
    EmbeddedCacheError, EmbeddedFailure, EmbeddedRenderError, EmbeddedRenderPayload,
    EmbeddedRenderRequest, EmbeddedRequestResult, EmbeddedResourceCache, EmbeddedResourceKind,
    EmbeddedRetryPolicy,
};
use yu_core::{ByteOffset, Revision, TextRange};

fn request(revision: u64, start: u64, source: &str) -> EmbeddedRenderRequest {
    let length = u64::try_from(source.len()).expect("short fixture source");
    EmbeddedRenderRequest::new(
        Revision::new(revision),
        TextRange::new(ByteOffset::new(start), ByteOffset::new(start + length))
            .expect("fixture range"),
        EmbeddedResourceKind::Math,
        source,
    )
    .expect("fixture request")
}

fn fail_job(
    cache: &mut EmbeddedResourceCache,
    input: &EmbeddedRenderRequest,
    error: EmbeddedRenderError,
) -> EmbeddedFailure {
    assert_eq!(cache.request(input.clone()), EmbeddedRequestResult::Pending);
    let job = cache.pending().expect("one queued job");
    assert!(cache.pending().is_none(), "duplicate worker submission");
    let result = cache
        .complete(job, input.revision(), Err(error))
        .expect("current completion");
    let EmbeddedRequestResult::Failed(failure) = result else {
        panic!("expected failed completion");
    };
    failure
}

fn advance_to_retry(cache: &mut EmbeddedResourceCache, input: &EmbeddedRenderRequest) {
    let due = cache.failure(input.key()).expect("retry metadata").next_retry_tick();
    let now = cache.retry_tick();
    assert!((now..=now + 64).contains(&due), "bounded fixture delay");
    for _ in now..due {
        cache.advance_retry_clock();
    }
}

#[test]
fn transient_roundtrips_stop_at_default_limit_and_preserve_backoff() {
    for error in [EmbeddedRenderError::Worker, EmbeddedRenderError::Render] {
        let mut cache = EmbeddedResourceCache::new();
        let input = request(11, 0, "x^2");
        for (attempt, delay) in [(1, 2), (2, 4), (3, 8)] {
            let failure = fail_job(&mut cache, &input, error);
            assert_eq!(failure.attempts(), attempt, "retry history was reset");
            assert_eq!(failure.kind(), error.failure_kind());
            assert_eq!(failure.next_retry_tick(), cache.retry_tick() + delay);
            assert_eq!(failure.is_exhausted(cache.retry_policy()), attempt == 3);
            for _ in 0..delay {
                assert_eq!(
                    cache.request(input.clone()),
                    EmbeddedRequestResult::Failed(failure.clone())
                );
                assert!(cache.pending().is_none(), "retry before its deadline");
                cache.advance_retry_clock();
            }
        }
        for _ in 0..128 {
            cache.advance_retry_clock();
            let EmbeddedRequestResult::Failed(failure) = cache.request(input.clone()) else {
                panic!("exhausted source must not launch another job");
            };
            assert_eq!(failure.attempts(), 3);
            assert!(cache.pending().is_none());
        }
        assert_eq!(cache.failure_count(), 1);
        assert!(cache.is_empty());
    }
}

#[test]
fn exponential_backoff_caps_without_resetting_attempts() {
    let mut cache = EmbeddedResourceCache::new();
    cache.set_retry_policy(EmbeddedRetryPolicy::new(5, 2, 3));
    let input = request(1, 0, "x^2");
    for (attempt, delay) in [(1, 2), (2, 3), (3, 3), (4, 3), (5, 3)] {
        let failure = fail_job(&mut cache, &input, EmbeddedRenderError::Worker);
        assert_eq!(failure.attempts(), attempt);
        assert_eq!(failure.next_retry_tick(), cache.retry_tick() + delay);
        if attempt < 5 {
            advance_to_retry(&mut cache, &input);
        } else {
            assert!(failure.is_exhausted(cache.retry_policy()));
        }
    }
}

#[test]
fn queued_and_running_retry_polls_keep_history_and_submit_only_once() {
    let mut cache = EmbeddedResourceCache::new();
    let input = request(1, 0, "x^2");
    let first = fail_job(&mut cache, &input, EmbeddedRenderError::Worker);
    advance_to_retry(&mut cache, &input);
    for start in 1..=64 {
        assert_eq!(
            cache.request(request(1, start, "x^2")),
            EmbeddedRequestResult::Pending
        );
        assert_eq!(cache.failure(input.key()), Some(&first));
    }
    let job = cache.pending().expect("one retry");
    assert_eq!(job.source_range().start(), ByteOffset::new(64));
    for _ in 0..64 {
        assert_eq!(cache.request(input.clone()), EmbeddedRequestResult::Pending);
        assert!(cache.pending().is_none(), "in-flight retry duplicated");
        assert_eq!(cache.failure(input.key()), Some(&first));
    }
    let EmbeddedRequestResult::Failed(second) = cache
        .complete(job, input.revision(), Err(EmbeddedRenderError::Render))
        .expect("retry completion")
    else {
        panic!("expected failed retry");
    };
    assert_eq!(second.attempts(), 2);
    assert_eq!(second.kind(), EmbeddedRenderError::Render.failure_kind());
}

#[test]
fn successful_retry_clears_failures_and_keeps_reusable_publication() {
    let mut cache = EmbeddedResourceCache::new();
    let input = request(1, 0, "x^2");
    fail_job(&mut cache, &input, EmbeddedRenderError::Worker);
    advance_to_retry(&mut cache, &input);
    assert_eq!(cache.request(input.clone()), EmbeddedRequestResult::Pending);
    let job = cache.pending().expect("retry job");
    let payload = EmbeddedRenderPayload::svg(2, 3, "<svg/>").expect("payload");
    let EmbeddedRequestResult::Ready(publication) = cache
        .complete(job, input.revision(), Ok(payload.clone()))
        .expect("successful retry")
    else {
        panic!("expected publication");
    };
    assert_eq!(cache.failure_count(), 0);
    assert_eq!(cache.len(), 1);
    assert_eq!(
        cache.request(input),
        EmbeddedRequestResult::Ready(publication.clone())
    );
    cache.retain_revision(Revision::new(2));
    let rebased = request(2, 20, "x^2");
    let EmbeddedRequestResult::Ready(reused) = cache.request(rebased.clone()) else {
        panic!("successful payload must remain reusable across revisions");
    };
    assert_eq!(reused.generation(), publication.generation());
    assert_eq!(reused.revision(), rebased.revision());
    assert_eq!(reused.source_range(), rebased.source_range());
    assert_eq!(reused.payload(), &payload);
    assert!(cache.pending().is_none());
}

#[test]
fn late_old_success_or_failure_cannot_reset_a_new_revisions_retry() {
    for old_succeeds in [false, true] {
        let mut cache = EmbeddedResourceCache::new();
        let old = request(1, 0, "x^2");
        fail_job(&mut cache, &old, EmbeddedRenderError::Worker);
        advance_to_retry(&mut cache, &old);
        assert_eq!(cache.request(old), EmbeddedRequestResult::Pending);
        let old_job = cache.pending().expect("old in-flight retry");
        cache.retain_revision(Revision::new(2));
        assert_eq!(cache.failure_count(), 0);
        let fresh = request(2, 10, "x^2");
        let first = fail_job(&mut cache, &fresh, EmbeddedRenderError::Worker);
        assert_eq!(first.attempts(), 1, "new revision needs a fresh budget");
        advance_to_retry(&mut cache, &fresh);
        assert_eq!(cache.request(fresh.clone()), EmbeddedRequestResult::Pending);
        let fresh_job = cache.pending().expect("current in-flight retry");
        let old_result = if old_succeeds {
            Ok(EmbeddedRenderPayload::svg(2, 3, "<svg/>").expect("payload"))
        } else {
            Err(EmbeddedRenderError::Worker)
        };
        assert!(matches!(
            cache.complete(old_job, fresh.revision(), old_result),
            Err(EmbeddedCacheError::StaleRevision { .. })
        ));
        assert_eq!(cache.failure(fresh.key()), Some(&first));
        assert_eq!(cache.request(fresh.clone()), EmbeddedRequestResult::Pending);
        assert!(cache.pending().is_none(), "stale completion released a new owner");
        let EmbeddedRequestResult::Failed(second) = cache
            .complete(fresh_job, fresh.revision(), Err(EmbeddedRenderError::Worker))
            .expect("current retry completion")
        else {
            panic!("expected current failure");
        };
        assert_eq!(second.attempts(), 2);
        assert_eq!(second.revision(), fresh.revision());
        assert!(cache.is_empty(), "stale success entered the new cache");
    }
}

#[test]
fn invalid_and_unsupported_sources_never_auto_retry() {
    for error in [EmbeddedRenderError::InvalidSource, EmbeddedRenderError::Unsupported] {
        let mut cache = EmbeddedResourceCache::new();
        let input = request(1, 0, "x^2");
        let first = fail_job(&mut cache, &input, error);
        assert_eq!(first.attempts(), 1);
        assert_eq!(first.next_retry_tick(), u64::MAX);
        for _ in 0..128 {
            cache.advance_retry_clock();
            assert_eq!(
                cache.request(input.clone()),
                EmbeddedRequestResult::Failed(first.clone())
            );
            assert!(cache.pending().is_none());
        }
    }
}

#[test]
fn zero_delay_and_single_attempt_policies_still_have_a_hard_limit() {
    for requested_limit in [0, 1, 2, 3] {
        let policy = EmbeddedRetryPolicy::new(requested_limit, 0, 0);
        let mut cache = EmbeddedResourceCache::new();
        cache.set_retry_policy(policy);
        let input = request(1, 0, "x^2");
        for attempt in 1..=policy.max_attempts() {
            assert_eq!(
                fail_job(&mut cache, &input, EmbeddedRenderError::Worker).attempts(),
                attempt
            );
        }
        for _ in 0..64 {
            let EmbeddedRequestResult::Failed(failure) = cache.request(input.clone()) else {
                panic!("zero delay cannot bypass the attempt limit");
            };
            assert_eq!(failure.attempts(), policy.max_attempts());
            assert!(cache.pending().is_none());
        }
    }
}

#[test]
fn exhausted_budget_is_local_to_its_resource_and_document_cache() {
    let mut first_document = EmbeddedResourceCache::new();
    first_document.set_retry_policy(EmbeddedRetryPolicy::new(2, 0, 0));
    let mut second_document = EmbeddedResourceCache::new();
    let input = request(1, 0, "x^2");
    fail_job(&mut first_document, &input, EmbeddedRenderError::Worker);
    let exhausted = fail_job(&mut first_document, &input, EmbeddedRenderError::Worker);
    assert_eq!(exhausted.attempts(), 2);
    assert!(exhausted.is_exhausted(first_document.retry_policy()));
    assert_eq!(
        fail_job(&mut second_document, &input, EmbeddedRenderError::Worker).attempts(),
        1
    );
    let other = request(1, 10, "y^2");
    assert_eq!(
        fail_job(&mut first_document, &other, EmbeddedRenderError::Worker).attempts(),
        1
    );
    assert_eq!(
        first_document.request(input),
        EmbeddedRequestResult::Failed(exhausted)
    );
    assert_eq!(first_document.failure_count(), 2);
    assert_eq!(second_document.failure_count(), 1);
    assert!(first_document.pending().is_none());
    assert!(second_document.pending().is_none());
}
