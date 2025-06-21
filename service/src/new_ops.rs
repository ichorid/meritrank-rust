//  FIXME: Code duplication, see Request and Response types in state_manager.rs.

use bincode::{Decode, Encode};

#[derive(Debug, Encode, Decode, Eq, PartialEq)]
pub enum ServiceRequestOpcode {
  ReadScores,
  WriteEdge,
}
impl ServiceRequestOpcode {
  pub fn is_read(&self) -> bool {
    match self {
      ServiceRequestOpcode::ReadScores => true,
      ServiceRequestOpcode::WriteEdge => false,
    }
  }
}

pub type SubgraphName = String;
pub type NodeName = String;
#[derive(Debug, Encode, Decode)]
pub struct Request {
  pub subgraph_name: SubgraphName,
  pub opcode:        ServiceRequestOpcode,
  pub ego:           NodeName,
  pub score_options: ScoreOptions,
}

#[derive(Debug, Encode, Decode)]
pub struct Response {
  pub response: u64,
}

#[derive(Debug, Encode, Decode)]
struct ScoreOptions {
    hide_personal: bool,
    score_lt: f64,
    score_lte: bool,
    score_gt: f64,
    score_gte: bool,
    index: u32,
    count: u32,
}

impl Default for ScoreOptions {
    fn default() -> Self {
        ScoreOptions {
            hide_personal: false,
            score_lt: f64::MAX,
            score_lte: true,
            score_gt: f64::MIN,
            score_gte: true,
            index: 0,
            count: u32::MAX,
        }
    }
}
