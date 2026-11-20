//! The agent module: everything that exists because a model, not a person, is on the other
//! end of the request.
//!
//! `protocol` is what an agent may send, `staging` is the lifecycle that turns a proposal
//! into rows only after a human has said so. The MCP transport and the tool handlers are
//! not here yet — they arrive with the `/mcp` mount, and nothing in this module depends on
//! them.
//!
//! The invariant the rest of the design leans on: **the write and the decision to write are
//! different requests with different credentials.** An agent holds a token and can stage;
//! only a session cookie can approve. That is why the state lives in `agent_staged_batch`
//! and not in the conversation — a status the model can describe is a status the model can
//! invent.

pub mod protocol;
pub mod staging;
