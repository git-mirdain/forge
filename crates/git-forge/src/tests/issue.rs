use crate::Error;
use crate::issue::IssueState;

#[test]
fn state_from_str_valid() {
    assert_eq!("open".parse::<IssueState>().unwrap(), IssueState::Open);
    assert_eq!("closed".parse::<IssueState>().unwrap(), IssueState::Closed);
}

#[test]
fn state_from_str_invalid() {
    let err = "pending".parse::<IssueState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));

    let err = "Open".parse::<IssueState>().unwrap_err(); // case-sensitive
    assert!(matches!(err, Error::InvalidState(_)));

    let err = "".parse::<IssueState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));
}

#[test]
fn state_as_str() {
    assert_eq!(IssueState::Open.as_str(), "open");
    assert_eq!(IssueState::Closed.as_str(), "closed");
}
