//! The judgment a survey asks of each sampled picture: is it usable for
//! mapping, does it show a hazard near the camera, and what ground does
//! it show. The questions are fixed so that results compare across models.

use serde_json::json;

use crate::{DecisionRequest, DecisionResponse, Image, Question};

/// What the survey knows about one picture.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameContext {
    /// The task the picture belongs to.
    pub task_id: String,
    /// Height of the camera above the ground, metres.
    pub height_m: f64,
    /// The camera points straight down.
    pub nadir: bool,
}

/// The ground cover options, in the order the question lists them.
pub const COVER_OPTIONS: [(&str, &str); 5] = [
    ("open", "fields, lawns or bare ground"),
    ("built", "buildings and roofs"),
    ("roads", "roads, paths or parking"),
    ("trees", "woodland or tree canopy"),
    ("water", "water"),
];

/// One picture's judgment.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameJudgment {
    /// Probability that the picture is usable for mapping.
    pub usable: f64,
    /// Probability that it shows a hazard near the camera.
    pub hazard: f64,
    /// The most probable ground cover.
    pub cover: Option<String>,
}

/// The request for one JPEG picture.
#[must_use]
pub fn frame_request(jpeg: Vec<u8>, context: &FrameContext) -> DecisionRequest {
    DecisionRequest {
        state: json!({
            "picture": "one frame from a drone camera during an aerial survey",
            "camera": if context.nadir { "pointing straight down" } else { "tilted" },
            "height_above_ground_m": context.height_m.round(),
            "task": context.task_id,
        }),
        questions: vec![
            (
                "usable".to_owned(),
                Question::YesNo {
                    instructions:
                        "Is this aerial picture usable for mapping the ground: in focus, \
                        neither too dark nor washed out, and showing the ground clearly?"
                            .to_owned(),
                },
            ),
            (
                "hazard".to_owned(),
                Question::YesNo {
                    instructions:
                        "Does the picture show a hazard to a low-flying drone close to the \
                        camera, such as a tower, power line, crane or other tall structure?"
                            .to_owned(),
                },
            ),
            (
                "cover".to_owned(),
                Question::Choice {
                    instructions: "What does most of the picture show?".to_owned(),
                    criteria: COVER_OPTIONS
                        .iter()
                        .map(|(name, meaning)| ((*name).to_owned(), (*meaning).to_owned()))
                        .collect(),
                },
            ),
        ],
        images: vec![Image {
            media_type: "image/jpeg".to_owned(),
            bytes: jpeg,
        }],
    }
}

/// Reads the answer to [`frame_request`].
#[must_use]
pub fn read_frame(response: &DecisionResponse) -> Option<FrameJudgment> {
    Some(FrameJudgment {
        usable: response.yes_no("usable")?,
        hazard: response.yes_no("hazard")?,
        cover: response.choice("cover").map(str::to_owned),
    })
}
