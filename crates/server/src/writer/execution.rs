use super::*;
use kiarni_feeds::{Change, Dispatcher, Kind};

type Results = Vec<Result<Outcome, Code>>;
fn failed(group: &[Job], code: Code) -> (Results, bool) {
    (group.iter().map(|_| Err(code)).collect(), true)
}
fn feed_error(error: kiarni_feeds::Error) -> Code {
    match error {
        kiarni_feeds::Error::TooManySubscribers => Code::SubscriptionLimit,
        _ => Code::ServiceUnavailable,
    }
}

pub(super) fn execute(
    store: &Store,
    group: &[Job],
    limit: usize,
    dispatcher: &mut Dispatcher,
    after_commit: &mut impl FnMut() -> Result<(), Code>,
) -> (Results, bool) {
    if group[0].command.is_update() {
        let updates: Vec<_> = group
            .iter()
            .filter_map(|job| match &job.command {
                Command::Update {
                    collection,
                    id,
                    operation,
                    body,
                } => Some(updates::Update {
                    collection,
                    id,
                    operation: *operation,
                    body,
                }),
                _ => None,
            })
            .collect();
        let capture = match updates
            .iter()
            .map(|update| dispatcher.observes(update.collection))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(capture) => capture,
            Err(_) => return failed(group, Code::StorageUnavailable),
        };
        let mut publication = match dispatcher.reserve(group.len()) {
            Ok(publication) => publication,
            Err(error) => return failed(group, feed_error(error)),
        };
        let captured = match store.update_group_captured(&updates, limit, &capture) {
            Ok(receipt) => receipt,
            Err(_) => {
                publication.interrupt();
                return failed(group, Code::StorageUnavailable);
            }
        };
        let confirmed_at = std::time::Instant::now();
        if captured.receipt.committed && after_commit().is_err() {
            publication.interrupt();
            return failed(group, Code::StorageUnavailable);
        }
        let result = (|| -> Result<Results, Code> {
            let mut results = Vec::with_capacity(group.len());
            for ((update, outcome), transition) in updates
                .iter()
                .zip(captured.receipt.outcomes)
                .zip(captured.transitions)
            {
                if outcome.as_ref().is_ok_and(|candidate| candidate.changed()) {
                    if let Some(transition) = transition {
                        let change = Change::from_shared(
                            update.collection,
                            update.id,
                            Kind::Update,
                            Some(transition.before),
                            Some(transition.after),
                        )
                        .map_err(feed_error)?;
                        publication
                            .publish_confirmed_at(change, confirmed_at)
                            .map_err(feed_error)?;
                    } else {
                        publication
                            .advance_unobserved_confirmed(update.collection)
                            .map_err(feed_error)?;
                    }
                }
                results.push(
                    outcome
                        .map(Outcome::Updated)
                        .map_err(super::super::collections::storage_error),
                );
            }
            Ok(results)
        })();
        return match result {
            Ok(results) => {
                publication.finish();
                (results, false)
            }
            Err(code) => {
                publication.interrupt();
                failed(group, code)
            }
        };
    }
    let map = super::super::collections::storage_error;
    let result = match &group[0].command {
        Command::Subscribe { collection } => {
            store.collection(collection).map_err(map).and_then(|_| {
                dispatcher
                    .subscribe(collection)
                    .map(Outcome::Subscribed)
                    .map_err(feed_error)
            })
        }
        Command::CreateCollection { name } => store
            .create_collection(name)
            .map(|_| Outcome::Created)
            .map_err(map),
        Command::DropCollection { name } => store
            .collection(name)
            .and_then(|collection| store.drop_collection(&collection))
            .map(|_| {
                dispatcher.interrupt_collection(name);
                Outcome::Dropped
            })
            .map_err(map),
        Command::Insert { collection, body } => {
            let observed = match dispatcher.observes(collection) {
                Ok(value) => value,
                Err(_) => return failed(group, Code::StorageUnavailable),
            };
            let mut publication = match dispatcher.reserve(1) {
                Ok(value) => value,
                Err(error) => return failed(group, feed_error(error)),
            };
            let result = store
                .collection(collection)
                .map_err(map)
                .and_then(|handle| super::super::documents::insert(store, &handle, body, limit));
            match result {
                Ok(candidate) => {
                    let confirmed_at = std::time::Instant::now();
                    let published = (|| -> Result<(), Code> {
                        after_commit()?;
                        if observed {
                            let change = Change::new(
                                collection,
                                candidate.id(),
                                Kind::Insert,
                                None,
                                Some(candidate.json()),
                            )
                            .map_err(feed_error)?;
                            publication
                                .publish_confirmed_at(change, confirmed_at)
                                .map_err(feed_error)?;
                        } else {
                            publication
                                .advance_unobserved_confirmed(collection)
                                .map_err(feed_error)?;
                        }
                        Ok(())
                    })();
                    if let Err(code) = published {
                        publication.interrupt();
                        return failed(group, code);
                    }
                    publication.finish();
                    Ok(Outcome::Inserted(candidate))
                }
                Err(code) => {
                    publication.finish();
                    Err(code)
                }
            }
        }
        Command::Delete { collection, id } => {
            let observed = match dispatcher.observes(collection) {
                Ok(value) => value,
                Err(_) => return failed(group, Code::StorageUnavailable),
            };
            let mut publication = match dispatcher.reserve(1) {
                Ok(value) => value,
                Err(error) => return failed(group, feed_error(error)),
            };
            let result = store
                .collection(collection)
                .map_err(map)
                .and_then(|handle| {
                    if observed {
                        store
                            .delete_document_captured(&handle, id)
                            .map(Some)
                            .map_err(|error| match error {
                                updates::GroupError::Aborted(error) => map(error),
                                _ => Code::StorageUnavailable,
                            })
                    } else {
                        store
                            .delete_document(&handle, id)
                            .map(|_| None)
                            .map_err(map)
                    }
                });
            match result {
                Ok(before) => {
                    let confirmed_at = std::time::Instant::now();
                    let published = (|| -> Result<(), Code> {
                        after_commit()?;
                        if let Some(before) = before {
                            let change = Change::from_shared(
                                collection,
                                id,
                                Kind::Delete,
                                Some(before),
                                None,
                            )
                            .map_err(feed_error)?;
                            publication
                                .publish_confirmed_at(change, confirmed_at)
                                .map_err(feed_error)?;
                        } else {
                            publication
                                .advance_unobserved_confirmed(collection)
                                .map_err(feed_error)?;
                        }
                        Ok(())
                    })();
                    if let Err(code) = published {
                        publication.interrupt();
                        return failed(group, code);
                    }
                    publication.finish();
                    Ok(Outcome::Deleted)
                }
                Err(code) => {
                    publication.finish();
                    Err(code)
                }
            }
        }
        Command::Update { .. } => Err(Code::InternalError),
    };
    let fatal = matches!(result, Err(Code::StorageUnavailable | Code::InternalError));
    (vec![result], fatal)
}
