use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, OpenApi, ToSchema};

/// Local proposal state mirror used by the REST indexer.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalState {
    Active,
    Passed,
    Rejected,
    Executed,
    Cancelled,
}

/// Summary data returned by the paginated proposals endpoint.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ProposalSummary {
    pub id: u64,
    pub title: String,
    pub state: ProposalState,
    pub quorum: i128,
    pub start_time: u64,
    pub end_time: u64,
}

/// Full proposal detail returned by ID.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ProposalDetail {
    pub id: u64,
    pub proposer: String,
    pub title: String,
    pub description: String,
    pub quorum: i128,
    pub votes_yes: i128,
    pub votes_no: i128,
    pub votes_abstain: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub state: ProposalState,
    pub execute_after: u64,
}

/// Voter history entry for query responses.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct VoteRecord {
    pub proposal_id: u64,
    pub voter: String,
    pub vote: VoteChoice,
    pub weight: i128,
}

/// Vote choice values emitted by the contract.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoteChoice {
    Yes,
    No,
    Abstain,
}

/// Event envelope used to feed the indexer.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ProposalCreated {
        id: u64,
        proposer: String,
        title: String,
        description: String,
        quorum: i128,
        start_time: u64,
        end_time: u64,
    },
    VoteCast {
        proposal_id: u64,
        voter: String,
        vote: VoteChoice,
        weight: i128,
    },
    ProposalFinalised {
        proposal_id: u64,
        state: ProposalState,
        execute_after: u64,
    },
    ProposalExecuted {
        proposal_id: u64,
    },
    ProposalCancelled {
        proposal_id: u64,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema, IntoParams)]
pub struct ProposalListParams {
    #[schema(example = 0)]
    pub offset: Option<u64>,
    #[schema(example = 50)]
    pub limit: Option<u64>,
    pub state: Option<ProposalState>,
}

/// In-memory event indexer for proposal and vote query patterns.
#[derive(Debug, Default)]
pub struct Indexer {
    proposals: HashMap<u64, ProposalDetail>,
    votes_by_proposal: HashMap<u64, Vec<VoteRecord>>,
    votes_by_voter: HashMap<String, Vec<VoteRecord>>,
}

impl Indexer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&mut self, event: Event) {
        match event {
            Event::ProposalCreated {
                id,
                proposer,
                title,
                description,
                quorum,
                start_time,
                end_time,
            } => {
                self.proposals.insert(
                    id,
                    ProposalDetail {
                        id,
                        proposer,
                        title,
                        description,
                        quorum,
                        votes_yes: 0,
                        votes_no: 0,
                        votes_abstain: 0,
                        start_time,
                        end_time,
                        state: ProposalState::Active,
                        execute_after: 0,
                    },
                );
            }
            Event::VoteCast {
                proposal_id,
                voter,
                vote,
                weight,
            } => {
                let record = VoteRecord {
                    proposal_id,
                    voter: voter.clone(),
                    vote: vote.clone(),
                    weight,
                };
                if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
                    match vote {
                        VoteChoice::Yes => proposal.votes_yes += weight,
                        VoteChoice::No => proposal.votes_no += weight,
                        VoteChoice::Abstain => proposal.votes_abstain += weight,
                    }
                }
                self.votes_by_proposal
                    .entry(proposal_id)
                    .or_default()
                    .push(record.clone());
                self.votes_by_voter.entry(voter).or_default().push(record);
            }
            Event::ProposalFinalised {
                proposal_id,
                state,
                execute_after,
            } => {
                if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
                    proposal.state = state;
                    proposal.execute_after = execute_after;
                }
            }
            Event::ProposalExecuted { proposal_id } => {
                if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
                    proposal.state = ProposalState::Executed;
                }
            }
            Event::ProposalCancelled { proposal_id } => {
                if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
                    proposal.state = ProposalState::Cancelled;
                }
            }
        }
    }

    pub fn list_proposals(
        &self,
        state: Option<ProposalState>,
        offset: u64,
        limit: u64,
    ) -> Vec<ProposalSummary> {
        let mut proposals: Vec<_> = self
            .proposals
            .values()
            .filter(|proposal| match &state {
                Some(filter_state) => &proposal.state == filter_state,
                None => true,
            })
            .cloned()
            .collect();

        proposals.sort_by_key(|proposal| proposal.id);

        proposals
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|proposal| ProposalSummary {
                id: proposal.id,
                title: proposal.title,
                state: proposal.state,
                quorum: proposal.quorum,
                start_time: proposal.start_time,
                end_time: proposal.end_time,
            })
            .collect()
    }

    pub fn get_proposal(&self, id: u64) -> Option<ProposalDetail> {
        self.proposals.get(&id).cloned()
    }

    pub fn get_proposal_votes(&self, id: u64) -> Vec<VoteRecord> {
        self.votes_by_proposal.get(&id).cloned().unwrap_or_default()
    }

    pub fn get_voter_votes(&self, voter: &str) -> Vec<VoteRecord> {
        self.votes_by_voter.get(voter).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::api::list_proposals,
        crate::api::get_proposal,
        crate::api::get_proposal_votes,
        crate::api::get_voter_votes,
        crate::api::openapi_json,
    ),
    components(schemas(
        ProposalState,
        ProposalSummary,
        ProposalDetail,
        VoteRecord,
        VoteChoice,
        ApiError,
        ProposalListParams
    ))
)]
pub struct ApiDoc;

pub mod api;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_ingest_and_query() {
        let mut indexer = Indexer::new();
        indexer.ingest(Event::ProposalCreated {
            id: 1,
            proposer: "GABC123".into(),
            title: "Test".into(),
            description: "A proposal".into(),
            quorum: 100,
            start_time: 1000,
            end_time: 2000,
        });

        indexer.ingest(Event::VoteCast {
            proposal_id: 1,
            voter: "GDEF456".into(),
            vote: VoteChoice::Yes,
            weight: 42,
        });

        let proposal = indexer.get_proposal(1).expect("proposal should exist");
        assert_eq!(proposal.votes_yes, 42);
        assert_eq!(proposal.state, ProposalState::Active);

        let votes = indexer.get_proposal_votes(1);
        assert_eq!(votes.len(), 1);
        assert_eq!(votes[0].voter, "GDEF456");

        let history = indexer.get_voter_votes("GDEF456");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].weight, 42);
    }

    #[test]
    fn test_list_proposals_with_state_filter() {
        let mut indexer = Indexer::new();
        indexer.ingest(Event::ProposalCreated {
            id: 1,
            proposer: "GABC123".into(),
            title: "Active".into(),
            description: "Active proposal".into(),
            quorum: 10,
            start_time: 1000,
            end_time: 2000,
        });
        indexer.ingest(Event::ProposalCreated {
            id: 2,
            proposer: "GABC123".into(),
            title: "Cancelled".into(),
            description: "Cancelled proposal".into(),
            quorum: 10,
            start_time: 1000,
            end_time: 2000,
        });
        indexer.ingest(Event::ProposalCancelled { proposal_id: 2 });

        let summaries = indexer.list_proposals(Some(ProposalState::Cancelled), 0, 10);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, 2);
    }
}
