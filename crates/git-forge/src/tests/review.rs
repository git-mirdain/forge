use crate::Error;
use crate::review::ReviewState;

#[test]
fn state_from_str_valid() {
    assert_eq!("open".parse::<ReviewState>().unwrap(), ReviewState::Open);
    assert_eq!("draft".parse::<ReviewState>().unwrap(), ReviewState::Draft);
    assert_eq!(
        "closed".parse::<ReviewState>().unwrap(),
        ReviewState::Closed
    );
    assert_eq!(
        "merged".parse::<ReviewState>().unwrap(),
        ReviewState::Merged
    );
}

#[test]
fn state_from_str_invalid() {
    let err = "pending".parse::<ReviewState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));
}

#[test]
fn state_as_str() {
    assert_eq!(ReviewState::Open.as_str(), "open");
    assert_eq!(ReviewState::Draft.as_str(), "draft");
    assert_eq!(ReviewState::Closed.as_str(), "closed");
    assert_eq!(ReviewState::Merged.as_str(), "merged");
}
