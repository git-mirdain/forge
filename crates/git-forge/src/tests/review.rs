use crate::Error;
use crate::review::ReviewState;

#[test]
fn state_from_str_valid() {
    assert_eq!("open".parse::<ReviewState>().unwrap(), ReviewState::Open);
    assert_eq!(
        "merged".parse::<ReviewState>().unwrap(),
        ReviewState::Merged
    );
    assert_eq!(
        "closed".parse::<ReviewState>().unwrap(),
        ReviewState::Closed
    );
}

#[test]
fn state_from_str_invalid() {
    let err = "pending".parse::<ReviewState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));

    let err = "Open".parse::<ReviewState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));
}

#[test]
fn state_as_str() {
    assert_eq!(ReviewState::Open.as_str(), "open");
    assert_eq!(ReviewState::Merged.as_str(), "merged");
    assert_eq!(ReviewState::Closed.as_str(), "closed");
}
