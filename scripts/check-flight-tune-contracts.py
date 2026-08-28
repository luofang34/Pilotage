#!/usr/bin/env python3
"""Check simulator-neutral tuning manifests and Rust contract identifiers."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path
from typing import Any

FORBIDDEN_PACKAGES = {"flight-tune-xplane", "pilotage-xplane-trial"}
FORBIDDEN_IDENTIFIERS = (
    "xplane",
    "acf",
    "aircraftfile",
    "trialplugin",
    "bridgeplugin",
    "weatherplugin",
    "hostapplicationid",
    "sdkversion",
)
CAMPAIGN_SHARED_TYPES = {
    "CampaignBudgetLimit",
    "ExecutionTarget",
    "CampaignPurpose",
    "PinnedFile",
    "SearchGroupConfig",
    "SearchGroupKind",
    "CampaignConfig",
    "TrainingGuardScenarioConfig",
    "TrainingSuiteConfig",
}
CAMPAIGN_ADAPTER_TYPE_ALLOWLIST = frozenset(
    {
        ("tools/flight-tune-campaign/src/config.rs", "XPlaneCampaignConfig"),
        ("tools/flight-tune-campaign/src/config.rs", "XPlaneSupportBundleConfig"),
        ("tools/flight-tune-campaign/src/config.rs", "XPlaneRuntimePluginConfig"),
        ("tools/flight-tune-campaign/src/config.rs", "AviateCampaignConfig"),
        (
            "tools/flight-tune-campaign/src/deployment/launch/readback.rs",
            "AviateXPlaneLaunchBinding",
        ),
        (
            "tools/flight-tune-campaign/src/preparation/machine.rs",
            "XPlaneMachinePaths",
        ),
        (
            "tools/flight-tune-campaign/src/resource_admission/canary.rs",
            "ResourceXPlaneProducerIdentity",
        ),
        (
            "tools/flight-tune-campaign/src/runtime_launcher/xplane/error.rs",
            "XPlaneCampaignRuntimeError",
        ),
    }
)
CAMPAIGN_SHARED_IDENTIFIER_ALLOWLIST = frozenset(
    {
        (
            "tools/flight-tune-campaign/src/config.rs",
            "CampaignConfig",
            "field",
            "aviate_xplane_contract",
        ),
        (
            "tools/flight-tune-campaign/src/config.rs",
            "CampaignConfig",
            "field",
            "xplane",
        ),
        (
            "tools/flight-tune-campaign/src/config.rs",
            "CampaignConfig",
            "field_type:xplane",
            "XPlaneCampaignConfig",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/error.rs",
            "DeploymentError",
            "variant",
            "XPlaneAttestation",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/error.rs",
            "DeploymentError",
            "variant",
            "XPlaneIdentity",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/graph.rs",
            "VerifiedDeploymentGraph",
            "field",
            "xplane_primary",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/graph.rs",
            "VerifiedDeploymentGraph",
            "field",
            "xplane_support",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/launch.rs",
            "ResolvedDeployment",
            "field",
            "xplane_runtime",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/launch.rs",
            "ResolvedDeployment",
            "field",
            "aviate_xplane_binding",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/launch.rs",
            "ResolvedDeployment",
            "field_type:aviate_xplane_binding",
            "AviateXPlaneLaunchBinding",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/manifest.rs",
            "DeploymentManifest",
            "field",
            "xplane_runtime",
        ),
        (
            "tools/flight-tune-campaign/src/environment.rs",
            "EnvironmentError",
            "variant",
            "XPlane",
        ),
        (
            "tools/flight-tune-campaign/src/environment.rs",
            "EnvironmentError",
            "variant",
            "XPlaneBackend",
        ),
        (
            "tools/flight-tune-campaign/src/error.rs",
            "CampaignError",
            "variant",
            "XPlaneRegistry",
        ),
        (
            "tools/flight-tune-campaign/src/error.rs",
            "CampaignError",
            "variant",
            "XPlaneManifest",
        ),
        (
            "tools/flight-tune-campaign/src/plan.rs",
            "CampaignPlan",
            "field",
            "xplane",
        ),
        (
            "tools/flight-tune-campaign/src/preparation/machine.rs",
            "CampaignMachinePaths",
            "field",
            "xplane",
        ),
        (
            "tools/flight-tune-campaign/src/preparation/machine.rs",
            "CampaignMachinePaths",
            "field_type:xplane",
            "XPlaneMachinePaths",
        ),
        (
            "tools/flight-tune-campaign/src/resource_admission/canary.rs",
            "ResourceProducerIdentity",
            "field",
            "xplane",
        ),
        (
            "tools/flight-tune-campaign/src/resource_admission/canary.rs",
            "ResourceProducerIdentity",
            "field_type:xplane",
            "ResourceXPlaneProducerIdentity",
        ),
    }
)
CAMPAIGN_SIMULATOR_ALIAS_LIMITS = {
    (
        "tools/flight-tune-campaign/src/bin/flight_tune_campaign.rs",
        "CampaignRuntimeError",
        "XPlaneCampaignRuntimeError",
    ): 1,
    (
        "tools/flight-tune-campaign/src/bin/flight_tune_campaign_present.rs",
        "PresentationRuntimeError",
        "XPlaneCampaignRuntimeError",
    ): 1,
    (
        "tools/flight-tune-campaign/src/bin/flight_tune_deploy.rs",
        "DeploymentRuntimeError",
        "XPlaneCampaignRuntimeError",
    ): 1,
    (
        "tools/flight-tune-campaign/src/bin/flight_tune_deploy.rs",
        "PostInstallRuntimeError",
        "XPlaneCampaignRuntimeError",
    ): 1,
    (
        "tools/flight-tune-campaign/src/deployment/launch/aviate_xplane.rs",
        "Running",
        "AviateXPlaneProcess",
    ): 1,
    (
        "tools/flight-tune-campaign/src/live.rs",
        "Backend",
        "XPlaneTuningBackend",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "Environment",
        "AviateXPlaneEnvironment",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "Error",
        "XPlaneCampaignRuntimeError",
    ): 2,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "Guard",
        "AviateXPlaneRuntimeGuard",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "VerifiedSession",
        "VerifiedXPlaneCampaignSession",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "XPlaneLease",
        "XPlaneCampaignRuntimeGuard",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "XPlaneLease",
        "XPlaneLease",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
        "XPlaneLease",
        "XPlaneRuntimePlatform",
    ): 2,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "Environment",
        "AviateXPlaneEnvironment",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "Error",
        "XPlaneCampaignRuntimeError",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "Listener",
        "XPlaneTrialListener",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "PlatformProbe",
        "XPlaneProbe",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "PlatformProbe",
        "XPlaneRuntimePlatform",
    ): 4,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "Process",
        "XPlaneProcess",
    ): 1,
    (
        "tools/flight-tune-campaign/src/runtime_launcher/xplane/platform.rs",
        "Session",
        "VerifiedXPlaneCampaignSession",
    ): 1,
}
CAMPAIGN_SIMULATOR_PUBLIC_USE_ALLOWLIST = frozenset(
    {
        (
            "tools/flight-tune-campaign/src/deployment.rs",
            "useexercise::{AviateXPlaneExerciseDomain,DEPLOYMENT_EXERCISE_FINAL_RECEIPT_SCHEMA_VERSION,DEPLOYMENT_EXERCISE_HEAD_SCHEMA_VERSION,DEPLOYMENT_EXERCISE_INTENT_SCHEMA_VERSION,DEPLOYMENT_EXERCISE_PLAN_SCHEMA_VERSION,DEPLOYMENT_EXERCISE_RESULT_SCHEMA_VERSION,DeploymentExercise,DeploymentExerciseActivationRequest,DeploymentExerciseCommandReceipt,DeploymentExerciseDomain,DeploymentExerciseError,DeploymentExerciseExpectedGeneration,DeploymentExerciseFinalReceipt,DeploymentExerciseHead,DeploymentExerciseIntent,DeploymentExerciseLiveState,DeploymentExerciseLiveStateKind,DeploymentExerciseMutationPermit,DeploymentExercisePlan,DeploymentExercisePrepareRequest,DeploymentExerciseResult,DeploymentExerciseRuntimeIdentity,DeploymentExerciseStatus,DeploymentExerciseStep,DeploymentExerciseStepOutcome,PreparedExerciseMutation,};",
        ),
        (
            "tools/flight-tune-campaign/src/deployment.rs",
            "uselaunch::{ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DeploymentRuntimeReadback,ResolvedDeployment,SIMULATOR_LAUNCH_RECEIPT_SCHEMA_VERSION,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,SimulatorLaunchReceipt,VerifiedSimulatorDeployment,publish_simulator_launch_release_blocking,simulator_launch_released_blocking,};",
        ),
        (
            "tools/flight-tune-campaign/src/deployment.rs",
            "uselaunch::{ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DeploymentRuntimeReadback,ResolvedDeployment,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,VerifiedSimulatorDeployment,};",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/exercise.rs",
            "usedomain::{AviateXPlaneExerciseDomain,DeploymentExerciseDomain,DeploymentExerciseMutationPermit,PreparedExerciseMutation,};",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/exercise/domain.rs",
            "useaviate_xplane::AviateXPlaneExerciseDomain;",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/launch.rs",
            "useaviate_xplane::{AviateXPlaneProcess,AviateXPlaneProcessAdapter};",
        ),
        (
            "tools/flight-tune-campaign/src/deployment/launch.rs",
            "usereadback::{AviateLaunchContract,AviateXPlaneLaunchBinding,DeploymentRuntimeReadback,SimulatorDeploymentReadback,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "useconfig::{AviateCampaignConfig,CAMPAIGN_SCHEMA_VERSION,CampaignBudgetLimit,CampaignConfig,CampaignPurpose,ExecutionTarget,PinnedFile,SearchGroupConfig,SearchGroupKind,TrainingGuardScenarioConfig,TrainingSuiteConfig,XPlaneCampaignConfig,XPlaneRuntimePluginConfig,XPlaneSupportBundleConfig,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "useconfig::{AviateCampaignConfig,CAMPAIGN_SCHEMA_VERSION,CampaignBudgetLimit,CampaignConfig,ExecutionTarget,PinnedFile,SearchGroupConfig,SearchGroupKind,TrainingGuardScenarioConfig,TrainingSuiteConfig,XPlaneCampaignConfig,XPlaneRuntimePluginConfig,XPlaneSupportBundleConfig,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "usedeployment::{ActivationRequest,ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DEPLOYMENT_MUTATION_RECEIPT_SCHEMA_VERSION,DEPLOYMENT_STATUS_SCHEMA_VERSION,DeploymentError,DeploymentInstaller,DeploymentManifest,DeploymentMutation,DeploymentMutationReceipt,DeploymentMutationWorkflowError,DeploymentMutationWorkflowResult,DeploymentObject,DeploymentReceipt,DeploymentRuntimeObject,DeploymentRuntimeReadback,DeploymentState,DeploymentStatusReceipt,LiveAttestationStatus,PostInstallCampaign,PreparedActivation,PreparedRollback,QualifiedDeployment,ResolvedDeployment,SIMULATOR_LAUNCH_RECEIPT_SCHEMA_VERSION,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,SimulatorLaunchReceipt,VerifiedSimulatorDeployment,publish_simulator_launch_release_blocking,release_activation_request_blocking,simulator_launch_released_blocking,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "usedeployment::{ActivationRequest,ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DEPLOYMENT_MUTATION_RECEIPT_SCHEMA_VERSION,DEPLOYMENT_STATUS_SCHEMA_VERSION,DeploymentError,DeploymentInstaller,DeploymentManifest,DeploymentMutation,DeploymentMutationReceipt,DeploymentObject,DeploymentReceipt,DeploymentRuntimeObject,DeploymentRuntimeReadback,DeploymentState,DeploymentStatusReceipt,LiveAttestationStatus,PostInstallCampaign,QualifiedDeployment,ResolvedDeployment,SIMULATOR_LAUNCH_RECEIPT_SCHEMA_VERSION,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,SimulatorLaunchReceipt,VerifiedSimulatorDeployment,publish_simulator_launch_release_blocking,release_activation_request_blocking,simulator_launch_released_blocking,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "usedeployment::{ActivationRequest,ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DEPLOYMENT_MUTATION_RECEIPT_SCHEMA_VERSION,DEPLOYMENT_STATUS_SCHEMA_VERSION,DeploymentError,DeploymentInstaller,DeploymentManifest,DeploymentMutation,DeploymentMutationReceipt,DeploymentObject,DeploymentReceipt,DeploymentRuntimeObject,DeploymentRuntimeReadback,DeploymentState,DeploymentStatusReceipt,LiveAttestationStatus,PostInstallCampaign,QualifiedDeployment,ResolvedDeployment,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,VerifiedSimulatorDeployment,release_activation_request_blocking,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "usedeployment::{ActivationRequest,ActiveDeployment,AviateLaunchContract,AviateXPlaneLaunchBinding,AviateXPlaneProcess,AviateXPlaneProcessAdapter,DeploymentError,DeploymentInstaller,DeploymentManifest,DeploymentObject,DeploymentReceipt,DeploymentRuntimeObject,DeploymentRuntimeReadback,PostInstallCampaign,QualifiedDeployment,ResolvedDeployment,SimulatorDeploymentLauncher,SimulatorDeploymentReadback,SimulatorLaunchAdapter,SimulatorLaunchGate,VerifiedSimulatorDeployment,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "uselive::AviateXPlaneEnvironment;",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "useplan::{AviateRuntimePlan,CAMPAIGN_RUN_BUDGET_SCHEMA_VERSION,CampaignPlan,CampaignRunBudget,LoadedFeelProfile,XPlaneRuntimePlan,};",
        ),
        (
            "tools/flight-tune-campaign/src/lib.rs",
            "usepreparation::{AviateMachinePaths,CAMPAIGN_MACHINE_PATHS_SCHEMA_VERSION,CampaignMachinePaths,PreparedCampaign,TypedMachinePathRenderer,XPlaneMachinePaths,pin_machine_file,prepare_campaign,prepare_campaign_for_machine,prepare_typed_machine_document,};",
        ),
        (
            "tools/flight-tune-campaign/src/preparation.rs",
            "usemachine::{AviateMachinePaths,CAMPAIGN_MACHINE_PATHS_SCHEMA_VERSION,CampaignMachinePaths,XPlaneMachinePaths,};",
        ),
        (
            "tools/flight-tune-campaign/src/resource_admission.rs",
            "usecanary::{RESOURCE_CAPACITY_CANARY_SCHEMA_VERSION,ResourceAviateProducerIdentity,ResourceCapacityCanary,ResourceCapacityCanaryReceipt,ResourceProducerIdentity,ResourceRuntimeIdentity,ResourceXPlaneProducerIdentity,seal_capacity_canary_blocking,verify_capacity_canary_blocking,};",
        ),
        (
            "tools/flight-tune-campaign/src/runtime_launcher.rs",
            "usexplane::{AviateXPlaneEnvironmentLauncher,AviateXPlaneRuntimeGuard,VerifiedXPlaneCampaignSession,XPlaneCampaignRuntimeConfig,XPlaneCampaignRuntimeError,};",
        ),
        (
            "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
            "useconfig::XPlaneCampaignRuntimeConfig;",
        ),
        (
            "tools/flight-tune-campaign/src/runtime_launcher/xplane.rs",
            "useerror::XPlaneCampaignRuntimeError;",
        ),
    }
)
CAMPAIGN_SIMULATOR_RESTRICTED_PUBLIC_USE_ALLOWLIST = frozenset(
    {
        (
            "tools/flight-tune-campaign/src/deployment/exercise/domain.rs",
            "pub(incrate::deployment)",
            "useaviate_xplane::{execute_exact_qualification_with_factory_blocking,prepare_exact_qualification_restore_blocking,};",
        ),
    }
)
SHARED_SIMULATOR_SOURCE_DIGESTS = {
    "tools/flight-tune/src/bin/flight-tune-feedback/feedback/arguments.rs": (
        "f13cd464fa5057e77c58c52850157ffd65f5743a98a0cbfc2d3c309cb111501d"
    ),
}
MANUAL_SERIALIZATION_SOURCE_DIGESTS = {
    "crates/pilotage-trial/src/digest.rs": (
        "05c3468ac925693ff225dff1276c1ed360e107528e603dd4a47156d2a36fbef7"
    ),
}
GENERATED_INCLUDE_ALLOWLIST = {
    "tools/flight-tune/src/flight_quality/identity.rs": (
        "include",
        "!",
        "(",
        "concat",
        "!",
        "(",
        "env",
        "!",
        "(",
        '"OUT_DIR"',
        ")",
        ",",
        '"/evaluator_source_identity.rs"',
        ")",
        ")",
    ),
    "tools/flight-tune-aviate/src/runtime/identity.rs": (
        "include",
        "!",
        "(",
        "concat",
        "!",
        "(",
        "env",
        "!",
        "(",
        '"OUT_DIR"',
        ")",
        ",",
        '"/runtime_source_identity.rs"',
        ")",
        ")",
    ),
}
GENERATED_INCLUDE_INPUTS = {
    "tools/flight-tune/src/flight_quality/identity.rs": (
        "tools/flight-tune/build.rs",
        "tools/flight-tune/build_support/evaluator_source_identity.rs",
    ),
    "tools/flight-tune-aviate/src/runtime/identity.rs": (
        "tools/flight-tune-aviate/build.rs",
        "tools/flight-tune-aviate/build_support/runtime_source_identity.rs",
    ),
}
GENERATED_INPUT_DIGESTS = {
    "tools/flight-tune/build.rs": frozenset(
        {
            "b56dd4918f6443ceb9804f5e4fb04d464268cb29ebeba76b7dc69a5a07a475cd",
            "f0424a1bc2c4cc2fe118246db46bd3b912f6c5c5460e487c53285f93629c0aac",
        }
    ),
    "tools/flight-tune/build_support/evaluator_source_identity.rs": frozenset(
        {"ce2adf92e3954858395151639c9bfd38234debd19157d1eaa94e96c47a223915"}
    ),
    "tools/flight-tune-aviate/build.rs": frozenset(
        {"843c7d7845f1342e41b4fc457654da121d2b22120e0ea3ff5cc3a6c1c1dc23f7"}
    ),
    "tools/flight-tune-aviate/build_support/runtime_source_identity.rs": frozenset(
        {"d29a99fcf0c26af600616584f874171cbbc90b723a37d4c92e6b706c91fd7497"}
    ),
}
GENERATED_BUILD_TARGETS = {
    "tools/flight-tune/Cargo.toml": (
        "tools/flight-tune/src/flight_quality/identity.rs",
        "tools/flight-tune/build.rs",
    ),
    "tools/flight-tune-aviate/Cargo.toml": (
        "tools/flight-tune-aviate/src/runtime/identity.rs",
        "tools/flight-tune-aviate/build.rs",
    ),
}
TEST_ONLY_MODULE_NAMES = {"tests", "test_support"}
NON_PRODUCTION_TARGET_KINDS = {"bench", "custom-build", "example", "test"}
CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
BINDGEN_TARGET_ENV_KEY = "BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_darwin"
BINDGEN_TARGET_ENV_VALUE = "--target=aarch64-apple-darwin"
CARGO_LOCK_PACKAGE_ALLOWLIST = frozenset(
    {
        (
            "errno",
            "0.3.14",
            CRATES_IO_SOURCE,
            "39cab71617ae0d63f51a36d69f866391735b51691dbda63cf6f96d042b63efeb",
        ),
        (
            "libc",
            "0.2.186",
            CRATES_IO_SOURCE,
            "68ab91017fe16c622486840e4c83c9a37afeff978bd239b5293d61ece587de66",
        ),
        (
            "libm",
            "0.2.16",
            CRATES_IO_SOURCE,
            "b6d2cec3eae94f9f509c767b45932f1ada8350c4bdb85af2fcab4a3c14807981",
        ),
        (
            "libproc",
            "0.14.11",
            CRATES_IO_SOURCE,
            "a54ad7278b8bc5301d5ffd2a94251c004feb971feba96c971ea4063645990757",
        ),
        (
            "rustix",
            "1.1.4",
            CRATES_IO_SOURCE,
            "b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190",
        ),
        (
            "serde",
            "1.0.228",
            CRATES_IO_SOURCE,
            "9a8e94ea7f378bd32cbbd37198a4a91436180c5bb472411e48b5ec2e2124ae9e",
        ),
        (
            "serde_derive",
            "1.0.228",
            CRATES_IO_SOURCE,
            "d540f220d3187173da220f885ab66608367b6574e925011a9353e4badda91d79",
        ),
        (
            "serde_json",
            "1.0.150",
            CRATES_IO_SOURCE,
            "e8014e44b4736ed0538adeecded0fce2a272f22dc9578a7eb6b2d9993c74cfb9",
        ),
        (
            "sha2",
            "0.10.9",
            CRATES_IO_SOURCE,
            "a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283",
        ),
        (
            "sha2",
            "0.11.0",
            CRATES_IO_SOURCE,
            "446ba717509524cb3f22f17ecc096f10f4822d76ab5c0b9822c5f9c284e825f4",
        ),
        (
            "sysctl",
            "0.7.1",
            CRATES_IO_SOURCE,
            "cca424247104946a59dacd27eaad296223b7feec3d168a6dd04585183091eb0b",
        ),
        (
            "thiserror",
            "2.0.18",
            CRATES_IO_SOURCE,
            "4288b5bcbc7920c07a1149a35cf9590a2aa808e0bc1eafaade0b80947865fbc4",
        ),
        (
            "thiserror-impl",
            "2.0.18",
            CRATES_IO_SOURCE,
            "ebc4ee7f67670e9b64d05fa4253e753e016c6c95ff35b89b7941d6b856dec1d5",
        ),
        (
            "tokio",
            "1.52.3",
            CRATES_IO_SOURCE,
            "8fc7f01b389ac15039e4dc9531aa973a135d7a4135281b12d7c1bc79fd57fffe",
        ),
        (
            "toml",
            "1.1.4+spec-1.1.0",
            CRATES_IO_SOURCE,
            "3aace63f4bbcdfc2c965b059de67119c89c4017a70d633be6c104910f67056f5",
        ),
        (
            "tracing",
            "0.1.44",
            CRATES_IO_SOURCE,
            "63e71662fa4b2a2c3a26f570f037eb95bb1f85397f3cd8076caed2f026a6d100",
        ),
        (
            "tracing-subscriber",
            "0.3.23",
            CRATES_IO_SOURCE,
            "cb7f578e5945fb242538965c2d0b04418d38ec25c79d160cd279bf0731c8d319",
        ),
    }
)
CARGO_LOCK_TRACKED_PACKAGES = frozenset(
    record[0] for record in CARGO_LOCK_PACKAGE_ALLOWLIST
)
CARGO_LOCK_REQUIRED_VERSIONS = {
    "errno": "0.3.14",
    "libc": "0.2.186",
    "libm": "0.2.16",
    "libproc": "0.14.11",
    "rustix": "1.1.4",
    "serde": "1.0.228",
    "serde_derive": "1.0.228",
    "serde_json": "1.0.150",
    "sha2": "0.10.9",
    "sysctl": "0.7.1",
    "thiserror": "2.0.18",
    "thiserror-impl": "2.0.18",
    "tokio": "1.52.3",
    "toml": "1.1.4+spec-1.1.0",
    "tracing": "0.1.44",
    "tracing-subscriber": "0.3.23",
}
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
GROUP_CLOSINGS = {"(": ")", "[": "]", "{": "}"}
SAFE_CONTRACT_ATTRIBUTES = {
    "allow",
    "cfg",
    "default",
    "deprecated",
    "derive",
    "doc",
    "error",
    "forbid",
    "from",
    "ignore",
    "inline",
    "must_use",
    "non_exhaustive",
    "path",
    "repr",
    "serde",
    "source",
    "test",
    "warn",
}
SAFE_CONTRACT_DERIVES = {
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "Deserialize",
    "Eq",
    "Error",
    "Hash",
    "Ord",
    "PartialEq",
    "PartialOrd",
    "Serialize",
}
SAFE_QUALIFIED_DERIVES = {
    ("serde", "Deserialize"),
    ("serde", "Serialize"),
    ("thiserror", "Error"),
}
SAFE_SERDE_OPTIONS = {
    "content",
    "default",
    "deny_unknown_fields",
    "rename",
    "rename_all",
    "serde",
    "skip",
    "skip_serializing_if",
    "tag",
}
RUST_SIMPLE_ESCAPES = {
    "0": "\0",
    "n": "\n",
    "r": "\r",
    "t": "\t",
    "\\": "\\",
    '"': '"',
    "'": "'",
}
DIRECT_DEPENDENCY_ALLOWLIST_HEADER = (
    "manifest",
    "dependency_key",
    "actual_package",
    "kind",
    "target",
    "optional",
    "source_kind",
    "source_ref",
    "default_features",
    "features",
    "version_req",
)


class ParseError(Exception):
    """A Rust source file cannot be tokenized or divided into type bodies."""


def report(message: str) -> None:
    """Write one stable guard diagnostic."""
    print(f"FORBIDDEN: {message}", file=sys.stderr)


def normalized(identifier: str) -> str:
    """Return the comparison form for one Rust identifier."""
    return identifier.lower().replace("_", "").replace("-", "")


def is_forbidden_identifier(identifier: str) -> bool:
    """Test one identifier against all simulator-specific name fragments."""
    value = normalized(identifier)
    return any(fragment in value for fragment in FORBIDDEN_IDENTIFIERS)


def read_direct_dependency_allowlist(
    root: Path,
) -> frozenset[tuple[str, ...]] | None:
    """Read the reviewed direct production dependency baseline."""
    path = root / "scripts/flight-tune-direct-dependency-allowlist.tsv"
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        report(f"direct dependency allowlist cannot be read: {error}")
        return None
    if not lines or tuple(lines[0].split("\t")) != DIRECT_DEPENDENCY_ALLOWLIST_HEADER:
        report("direct dependency allowlist has an invalid header")
        return None
    records = [tuple(line.split("\t")) for line in lines[1:]]
    if any(
        len(record) != len(DIRECT_DEPENDENCY_ALLOWLIST_HEADER) for record in records
    ):
        report("direct dependency allowlist has an invalid record")
        return None
    if records != sorted(set(records)):
        report("direct dependency allowlist is not sorted or has a duplicate")
        return None
    return frozenset(records)


def check_cargo_source_overrides(root: Path) -> bool:
    """Check Cargo source controls and the macOS bindgen target."""
    valid = True
    bindgen_target_is_exact = False
    manifest = root / "Cargo.toml"
    try:
        workspace_document = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        report(f"Cargo.toml cannot be parsed for source overrides: {error}")
        return False
    if workspace_document.get("patch") or workspace_document.get("replace"):
        report("Cargo.toml has an unreviewed dependency source override")
        valid = False
    for relative in (".cargo/config", ".cargo/config.toml"):
        path = root / relative
        if not path.exists():
            continue
        if not is_regular_file_without_symlinks(path, root):
            report(f"{relative} is not a regular in-workspace Cargo configuration")
            valid = False
            continue
        try:
            document = tomllib.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
            report(f"{relative} cannot be parsed for source overrides: {error}")
            valid = False
            continue
        if any(
            document.get(key)
            for key in ("include", "patch", "paths", "replace", "source")
        ):
            report(f"{relative} has an unreviewed dependency source override")
            valid = False
        if relative == ".cargo/config.toml":
            environment = document.get("env")
            bindgen_target_is_exact = (
                isinstance(environment, dict)
                and environment.get(BINDGEN_TARGET_ENV_KEY)
                == BINDGEN_TARGET_ENV_VALUE
            )
    if not bindgen_target_is_exact:
        report(
            f".cargo/config.toml must set {BINDGEN_TARGET_ENV_KEY} "
            f'to "{BINDGEN_TARGET_ENV_VALUE}"'
        )
        valid = False
    return valid


def check_cargo_lock_packages(root: Path, required_names: set[str]) -> bool:
    """Bind protected registry dependencies to reviewed lockfile records."""
    lock_path = root / "Cargo.lock"
    if not lock_path.exists() and not required_names:
        return True
    if not is_regular_file_without_symlinks(lock_path, root):
        report("Cargo.lock is missing or is not a regular in-workspace file")
        return False
    try:
        document = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        report(f"Cargo.lock cannot be parsed: {error}")
        return False
    packages = document.get("package")
    if not isinstance(packages, list):
        report("Cargo.lock has no package records")
        return False

    valid = True
    observed_versions: set[tuple[str, str]] = set()
    for package in packages:
        if not isinstance(package, dict):
            report("Cargo.lock has an invalid package record")
            valid = False
            continue
        name = package.get("name")
        if name not in CARGO_LOCK_TRACKED_PACKAGES:
            continue
        record = (
            name,
            package.get("version"),
            package.get("source"),
            package.get("checksum"),
        )
        if record not in CARGO_LOCK_PACKAGE_ALLOWLIST:
            report(f"Cargo.lock has an unreviewed registry identity for {name}")
            valid = False
        elif isinstance(record[1], str):
            observed_versions.add((name, record[1]))
    for name in sorted(required_names):
        version = CARGO_LOCK_REQUIRED_VERSIONS.get(name)
        if version is None:
            report(f"Cargo.lock has no reviewed version rule for {name}")
            valid = False
        elif (name, version) not in observed_versions:
            report(f"Cargo.lock has no reviewed registry identity for {name} {version}")
            valid = False
    return valid


def dependency_record(
    manifest: Path, dependency: dict[str, Any], root: Path
) -> tuple[str, ...] | None:
    """Return the stable identity of one direct Cargo dependency."""
    name = dependency.get("name")
    rename = dependency.get("rename")
    kind_value = dependency.get("kind")
    target_value = dependency.get("target")
    optional = dependency.get("optional")
    default_features = dependency.get("uses_default_features")
    features = dependency.get("features")
    version_req = dependency.get("req")
    if not (
        isinstance(name, str)
        and (rename is None or isinstance(rename, str))
        and (kind_value is None or isinstance(kind_value, str))
        and (target_value is None or isinstance(target_value, str))
        and isinstance(optional, bool)
        and isinstance(default_features, bool)
        and isinstance(features, list)
        and all(isinstance(feature, str) for feature in features)
        and isinstance(version_req, str)
    ):
        report(f"{manifest.relative_to(root)} has incomplete dependency metadata")
        return None
    source = dependency.get("source")
    dependency_path = dependency.get("path")
    if isinstance(source, str):
        source_kind = "source"
        source_ref = source
    elif isinstance(dependency_path, str):
        source_kind = "path"
        resolved_path = Path(dependency_path).resolve()
        try:
            source_ref = resolved_path.relative_to(root.resolve()).as_posix()
        except ValueError:
            source_ref = resolved_path.as_posix()
    else:
        report(f"{manifest.relative_to(root)} has a dependency without a source")
        return None
    return (
        manifest.relative_to(root).as_posix(),
        rename if rename is not None else name,
        name,
        "normal" if kind_value is None else kind_value,
        "" if target_value is None else target_value,
        str(optional).lower(),
        source_kind,
        source_ref,
        str(default_features).lower(),
        ",".join(sorted(features)),
        version_req,
    )


def target_kinds(target: dict[str, Any]) -> set[str]:
    """Return the declared Cargo kinds for one target."""
    kinds = target.get("kind", [])
    return {kind for kind in kinds if isinstance(kind, str)}


def check_package_targets(package: dict[str, Any], manifest: Path, root: Path) -> bool:
    """Bind production Cargo targets to the source paths that the guard scans."""
    valid = True
    source_root = manifest.parent / "src"
    targets = [
        target for target in package.get("targets", []) if isinstance(target, dict)
    ]
    for target in targets:
        kinds = target_kinds(target)
        if not kinds:
            report(f"{manifest.relative_to(root)} has a target without a kind")
            valid = False
            continue
        if kinds.issubset(NON_PRODUCTION_TARGET_KINDS):
            continue
        source_value = target.get("src_path")
        if not isinstance(source_value, str):
            report(f"{manifest.relative_to(root)} has a target without a source path")
            valid = False
            continue
        source = Path(source_value)
        if not path_is_within(source, source_root) or not is_production_path(
            source, source_root
        ):
            report(
                f"{manifest.relative_to(root)} has a production target outside its scanned source root"
            )
            valid = False
        elif not is_regular_file_without_symlinks(source, root):
            report(f"{manifest.relative_to(root)} has an unsafe production target path")
            valid = False

    relative = manifest.relative_to(root).as_posix()
    generated_target = GENERATED_BUILD_TARGETS.get(relative)
    custom_builds = [
        target for target in targets if "custom-build" in target_kinds(target)
    ]
    if generated_target is None:
        if custom_builds:
            report(
                f"{manifest.relative_to(root)} has an unreviewed custom build target"
            )
            valid = False
        return valid
    generated_artifacts = (
        generated_target[0],
        *GENERATED_INCLUDE_INPUTS.get(generated_target[0], ()),
    )
    if not custom_builds and not any(
        (root / artifact).exists() for artifact in generated_artifacts
    ):
        return valid
    expected_build = (root / generated_target[1]).resolve()
    actual_builds = {
        Path(build_source).resolve()
        for target in custom_builds
        if isinstance((build_source := target.get("src_path")), str)
    }
    if actual_builds != {expected_build}:
        report(f"{manifest.relative_to(root)} has an unreviewed custom build target")
        valid = False
    return valid


def check_manifest(
    manifest: Path,
    root: Path,
    direct_dependency_allowlist: frozenset[tuple[str, ...]],
    allowed_adapter_dependencies: frozenset[str] = frozenset(),
) -> tuple[bool, set[str]]:
    """Use Cargo's resolved dependency view for one neutral package."""
    if not manifest.is_file():
        return True, set()
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            str(manifest),
        ],
        check=False,
        capture_output=True,
        text=True,
        cwd=root,
    )
    relative = manifest.relative_to(root)
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()
        suffix = f": {detail[-1]}" if detail else ""
        report(f"{relative} cargo metadata failed{suffix}")
        return False, set()
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        report(f"{relative} cargo metadata is not valid JSON: {error}")
        return False, set()

    resolved = manifest.resolve()
    package = next(
        (
            item
            for item in metadata.get("packages", [])
            if Path(item.get("manifest_path", "")).resolve() == resolved
        ),
        None,
    )
    if package is None:
        report(f"{relative} is absent from cargo metadata")
        return False, set()

    valid = check_package_targets(package, manifest, root)
    registry_names: set[str] = set()
    for dependency in package.get("dependencies", []):
        name = dependency.get("name")
        rename = dependency.get("rename")
        if rename in {"serde", "thiserror"} and rename != name:
            report(f"{relative} aliases dependency {name} as protected crate {rename}")
            valid = False
        if name in {"serde", "thiserror"} and rename is not None:
            report(f"{relative} renames protected dependency {name}")
            valid = False
        if dependency.get("kind") == "dev":
            continue
        record = dependency_record(manifest, dependency, root)
        if record is None:
            valid = False
        else:
            if record[6] == "source":
                registry_names.add(record[2])
                if record[2] == "serde" and "derive" in record[9].split(","):
                    registry_names.add("serde_derive")
                if record[2] == "thiserror":
                    registry_names.add("thiserror-impl")
            if record not in direct_dependency_allowlist:
                dependency_key = record[1]
                report(
                    f"{relative} has unreviewed direct production dependency {dependency_key}"
                )
                valid = False
        if name not in FORBIDDEN_PACKAGES:
            continue
        canonical_adapter_dependency = (
            name in allowed_adapter_dependencies
            and dependency.get("rename") is None
            and dependency.get("kind") is None
            and dependency.get("optional") is False
            and dependency.get("target") is None
        )
        if not canonical_adapter_dependency:
            report(f"{relative} has noncanonical runtime dependency {name}")
            valid = False
    return valid, registry_names


def raw_string_end(source: str, start: int) -> int | None:
    """Return the end of a Rust raw string that starts at one byte offset."""
    prefix_length = 0
    if source.startswith("br", start):
        prefix_length = 2
    elif source.startswith("r", start):
        prefix_length = 1
    else:
        return None
    cursor = start + prefix_length
    hashes = 0
    while cursor < len(source) and source[cursor] == "#":
        hashes += 1
        cursor += 1
    if cursor >= len(source) or source[cursor] != '"':
        return None
    terminator = '"' + ("#" * hashes)
    end = source.find(terminator, cursor + 1)
    if end < 0:
        raise ParseError("an unterminated raw string")
    return end + len(terminator)


def quoted_end(source: str, start: int, quote: str) -> int:
    """Return the end of one escaped Rust string."""
    cursor = start + 1
    while cursor < len(source):
        if source[cursor] == "\\":
            cursor += 2
            continue
        if source[cursor] == quote:
            return cursor + 1
        cursor += 1
    raise ParseError("an unterminated string")


def character_end(source: str, start: int) -> int | None:
    """Return the end of one Rust character literal, but not a lifetime."""
    cursor = start + 1
    if cursor >= len(source) or source[cursor] == "\n":
        return None
    if source[cursor] == "\\":
        cursor += 1
        while cursor < len(source) and source[cursor] not in {"'", "\n"}:
            cursor += 1
    else:
        cursor += 1
    if cursor < len(source) and source[cursor] == "'":
        return cursor + 1
    return None


def rust_tokens(source: str, *, keep_literals: bool = False) -> list[str]:
    """Return Rust identifier and punctuation tokens without comments."""
    tokens: list[str] = []
    cursor = 0
    while cursor < len(source):
        if source[cursor].isspace():
            cursor += 1
            continue
        if source.startswith("//", cursor):
            end = source.find("\n", cursor + 2)
            cursor = len(source) if end < 0 else end + 1
            continue
        if source.startswith("/*", cursor):
            depth = 1
            cursor += 2
            while cursor < len(source) and depth > 0:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            if depth != 0:
                raise ParseError("an unterminated block comment")
            continue
        raw_end = raw_string_end(source, cursor)
        if raw_end is not None:
            if keep_literals:
                tokens.append(source[cursor:raw_end])
            cursor = raw_end
            continue
        if source[cursor] == '"':
            string_end = quoted_end(source, cursor, '"')
            if keep_literals:
                tokens.append(source[cursor:string_end])
            cursor = string_end
            continue
        if source[cursor] == "'":
            character_literal_end = character_end(source, cursor)
            if character_literal_end is not None:
                cursor = character_literal_end
                continue
        raw_identifier = re.match(r"r#([A-Za-z_][A-Za-z0-9_]*)", source[cursor:])
        if raw_identifier is not None:
            tokens.append(raw_identifier.group(1))
            cursor += len(raw_identifier.group(0))
            continue
        identifier = re.match(r"[A-Za-z_][A-Za-z0-9_]*", source[cursor:])
        if identifier is not None:
            tokens.append(identifier.group(0))
            cursor += len(identifier.group(0))
            continue
        tokens.append(source[cursor])
        cursor += 1
    return tokens


def group_end(tokens: list[str], start: int, opening: str, closing: str) -> int:
    """Return the token after one balanced group."""
    depth = 0
    for index in range(start, len(tokens)):
        if tokens[index] == opening:
            depth += 1
        elif tokens[index] == closing:
            depth -= 1
            if depth == 0:
                return index + 1
    raise ParseError(f"an unclosed {opening}{closing} group")


def statement_end(tokens: list[str], start: int) -> int:
    """Return the token after one top-level semicolon."""
    depths = {"(": 0, "[": 0, "{": 0}
    closing = {
        ")": "(",
        "]": "[",
        "}": "{",
    }
    for index in range(start, len(tokens)):
        token = tokens[index]
        if token == ";" and all(depth == 0 for depth in depths.values()):
            return index + 1
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
    return len(tokens)


def type_body_start(tokens: list[str], start: int) -> tuple[int, str] | None:
    """Find a braced or tuple body after one named type."""
    paren_depth = 0
    bracket_depth = 0
    angle_depth = 0
    has_where_clause = False
    for index in range(start, len(tokens)):
        token = tokens[index]
        if (
            token == "("
            and paren_depth == bracket_depth == angle_depth == 0
            and not has_where_clause
        ):
            return index, "("
        if token == "(":
            paren_depth += 1
        elif token == ")":
            paren_depth = max(0, paren_depth - 1)
        elif token == "[":
            bracket_depth += 1
        elif token == "]":
            bracket_depth = max(0, bracket_depth - 1)
        elif token == "<":
            angle_depth += 1
        elif token == ">":
            angle_depth = max(0, angle_depth - 1)
        elif token == "where" and paren_depth == bracket_depth == angle_depth == 0:
            has_where_clause = True
        elif token == "{" and paren_depth == bracket_depth == angle_depth == 0:
            return index, "{"
        elif token == ";" and paren_depth == bracket_depth == angle_depth == 0:
            return index, ";"
    raise ParseError("a type declaration has no body terminator")


def top_level_segments(tokens: list[str]) -> list[list[str]]:
    """Divide one type body at top-level commas."""
    segments: list[list[str]] = []
    current: list[str] = []
    depths = {"(": 0, "[": 0, "{": 0}
    closing = {")": "(", "]": "[", "}": "{"}
    for token in tokens:
        if token == "," and all(depth == 0 for depth in depths.values()):
            if current:
                segments.append(current)
            current = []
            continue
        current.append(token)
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
    if current:
        segments.append(current)
    return segments


def strip_prefix(tokens: list[str]) -> list[str]:
    """Remove field or variant attributes and visibility tokens."""
    cursor = 0
    while cursor + 1 < len(tokens) and tokens[cursor : cursor + 2] == ["#", "["]:
        cursor = group_end(tokens, cursor + 1, "[", "]")
    if cursor < len(tokens) and tokens[cursor] == "pub":
        cursor += 1
        if cursor < len(tokens) and tokens[cursor] == "(":
            cursor = group_end(tokens, cursor, "(", ")")
    return tokens[cursor:]


def named_field_identifiers(
    body: list[str], context_prefix: str = ""
) -> list[tuple[str, str]]:
    """Return named field identifiers from one braced body."""
    records: list[tuple[str, str]] = []
    for segment in top_level_segments(body):
        candidate = strip_prefix(segment)
        if ":" not in candidate:
            continue
        colon = candidate.index(":")
        names = [token for token in candidate[:colon] if IDENTIFIER.fullmatch(token)]
        if names:
            name = names[-1]
            records.append(("field", name))
            records.extend(
                simulator_type_identifiers(
                    candidate[colon + 1 :], f"field_type:{context_prefix}{name}"
                )
            )
    return records


def simulator_type_identifiers(
    tokens: list[str], context: str
) -> list[tuple[str, str]]:
    """Return simulator-specific identifiers from one contract type context."""
    return [
        (context, token)
        for token in tokens
        if IDENTIFIER.fullmatch(token) and is_forbidden_identifier(token)
    ]


def tuple_field_identifiers(
    body: list[str], context_prefix: str
) -> list[tuple[str, str]]:
    """Return simulator-specific type identifiers from tuple fields."""
    records: list[tuple[str, str]] = []
    for index, segment in enumerate(top_level_segments(body)):
        records.extend(
            simulator_type_identifiers(
                strip_prefix(segment), f"{context_prefix}:{index}"
            )
        )
    return records


def contract_identifiers(
    kind: str, body: list[str], opening: str
) -> list[tuple[str, str]]:
    """Return field or variant identifiers from one named type body."""
    if kind == "struct":
        return (
            named_field_identifiers(body)
            if opening == "{"
            else tuple_field_identifiers(body, "tuple_field_type")
        )

    records: list[tuple[str, str]] = []
    for segment in top_level_segments(body):
        candidate = strip_prefix(segment)
        if candidate and IDENTIFIER.fullmatch(candidate[0]):
            variant = candidate[0]
            records.append(("variant", variant))
            if len(candidate) > 1 and candidate[1] == "{":
                body_end = group_end(candidate, 1, "{", "}")
                records.extend(
                    named_field_identifiers(candidate[2 : body_end - 1], f"{variant}.")
                )
            elif len(candidate) > 1 and candidate[1] == "(":
                body_end = group_end(candidate, 1, "(", ")")
                records.extend(
                    tuple_field_identifiers(
                        candidate[2 : body_end - 1], f"variant_type:{variant}"
                    )
                )
    return records


def derives_serialization(attributes: list[list[str]]) -> bool:
    """Return true when one attribute set derives a Serde contract."""
    for attribute in attributes:
        if len(attribute) < 4 or attribute[0:2] != ["derive", "("]:
            continue
        for segment in top_level_segments(attribute[2:-1]):
            derive = derive_path(segment)
            if derive is not None and derive[-1] in {"Deserialize", "Serialize"}:
                return True
    return False


def public_types(
    tokens: list[str],
) -> list[tuple[str, str, list[tuple[str, str]], bool]]:
    """Return public named structs and enums with their contract identifiers."""
    records: list[tuple[str, str, list[tuple[str, str]], bool]] = []
    cursor = 0
    public = False
    attributes: list[list[str]] = []
    while cursor < len(tokens):
        if cursor + 1 < len(tokens) and tokens[cursor : cursor + 2] == ["#", "["]:
            end = group_end(tokens, cursor + 1, "[", "]")
            attributes.append(tokens[cursor + 2 : end - 1])
            cursor = end
            continue
        if tokens[cursor] == "pub":
            public = True
            cursor += 1
            if cursor < len(tokens) and tokens[cursor] == "(":
                cursor = group_end(tokens, cursor, "(", ")")
            continue
        if tokens[cursor] not in {"struct", "enum"}:
            public = False
            attributes = []
            cursor += 1
            continue
        kind = tokens[cursor]
        if cursor + 1 >= len(tokens) or not IDENTIFIER.fullmatch(tokens[cursor + 1]):
            raise ParseError(f"a public {kind} has no identifier")
        name = tokens[cursor + 1]
        body = type_body_start(tokens, cursor + 2)
        if body is None:
            public = False
            cursor += 2
            continue
        body_start, opening = body
        if opening == ";":
            body_end = body_start + 1
            body_identifiers: list[tuple[str, str]] = []
            declaration_end = body_end
            tail: list[str] = []
        else:
            closing = GROUP_CLOSINGS[opening]
            body_end = group_end(tokens, body_start, opening, closing)
            body_identifiers = contract_identifiers(
                kind, tokens[body_start + 1 : body_end - 1], opening
            )
            declaration_end = (
                statement_end(tokens, body_end) if opening == "(" else body_end
            )
            tail = tokens[body_end : declaration_end - 1]
        if public:
            identifiers = simulator_type_identifiers(
                tokens[cursor + 2 : body_start] + tail, "header_type"
            )
            identifiers.extend(body_identifiers)
            records.append(
                (
                    kind,
                    name,
                    identifiers,
                    derives_serialization(attributes),
                )
            )
        public = False
        attributes = []
        cursor = declaration_end
    return records


def is_production_path(path: Path, source_root: Path) -> bool:
    """Exclude explicit Rust test modules from a production source scan."""
    relative = path.relative_to(source_root)
    return (
        path.name not in {"tests.rs", "test_support.rs"}
        and "tests" not in relative.parts
        and "test_support" not in relative.parts
    )


def cfg_expression_requires_test(tokens: list[str]) -> bool:
    """Return true when one cfg expression cannot select a non-test build."""
    if tokens == ["test"]:
        return True
    if len(tokens) < 3 or tokens[1] != "(" or tokens[-1] != ")":
        return False
    arguments = top_level_segments(tokens[2:-1])
    if tokens[0] == "all":
        return any(cfg_expression_requires_test(argument) for argument in arguments)
    if tokens[0] == "any":
        return bool(arguments) and all(
            cfg_expression_requires_test(argument) for argument in arguments
        )
    return False


def attributes_require_test(attributes: list[list[str]]) -> bool:
    """Return true when one item has a cfg attribute that requires test."""
    for attribute in attributes:
        if (
            len(attribute) >= 4
            and attribute[0:2] == ["cfg", "("]
            and attribute[-1] == ")"
            and cfg_expression_requires_test(attribute[2:-1])
        ):
            return True
    return False


def raw_rust_string_value(token: str) -> str | None:
    """Decode a Rust raw string or raw byte string."""
    if token.startswith("br"):
        cursor = 2
    elif token.startswith("r"):
        cursor = 1
    else:
        return None
    hashes = 0
    while cursor < len(token) and token[cursor] == "#":
        hashes += 1
        cursor += 1
    if cursor >= len(token) or token[cursor] != '"':
        return None
    terminator = '"' + ("#" * hashes)
    if not token.endswith(terminator):
        return None
    return token[cursor + 1 : -len(terminator)]


def rust_unicode_escape(content: str, start: int) -> tuple[str, int] | None:
    """Decode one Rust Unicode escape after its opening brace."""
    end = content.find("}", start)
    if end < 0:
        return None
    digits = content[start:end].replace("_", "")
    if not digits or len(digits) > 6 or re.fullmatch(r"[0-9A-Fa-f]+", digits) is None:
        return None
    value = int(digits, 16)
    if value > 0x10FFFF or 0xD800 <= value <= 0xDFFF:
        return None
    return chr(value), end + 1


def escaped_rust_string_value(content: str) -> str | None:
    """Decode the escape forms that Rust permits in a string literal."""
    value: list[str] = []
    cursor = 0
    while cursor < len(content):
        if content[cursor] != "\\":
            value.append(content[cursor])
            cursor += 1
            continue
        cursor += 1
        if cursor >= len(content):
            return None
        escape = content[cursor]
        if escape in RUST_SIMPLE_ESCAPES:
            value.append(RUST_SIMPLE_ESCAPES[escape])
            cursor += 1
        elif escape == "x" and re.fullmatch(
            r"[0-9A-Fa-f]{2}", content[cursor + 1 : cursor + 3]
        ):
            value.append(chr(int(content[cursor + 1 : cursor + 3], 16)))
            cursor += 3
        elif escape == "u" and content[cursor + 1 : cursor + 2] == "{":
            decoded = rust_unicode_escape(content, cursor + 2)
            if decoded is None:
                return None
            character, cursor = decoded
            value.append(character)
        elif escape == "\n":
            cursor += 1
            while cursor < len(content) and content[cursor].isspace():
                cursor += 1
        else:
            return None
    return "".join(value)


def rust_string_value(token: str) -> str | None:
    """Decode a Rust string or byte string literal."""
    raw_value = raw_rust_string_value(token)
    if raw_value is not None:
        return raw_value
    literal = token[1:] if token.startswith(('b"', 'c"')) else token
    if len(literal) < 2 or not literal.startswith('"') or not literal.endswith('"'):
        return None
    return escaped_rust_string_value(literal[1:-1])


def path_attribute_value(attributes: list[list[str]]) -> str | None:
    """Return the path from one Rust path attribute."""
    for attribute in attributes:
        if len(attribute) == 3 and attribute[0:2] == ["path", "="]:
            return rust_string_value(attribute[2])
    return None


def item_attributes(tokens: list[str], start: int) -> tuple[list[list[str]], int]:
    """Read consecutive outer attributes before one Rust item."""
    attributes: list[list[str]] = []
    cursor = start
    while cursor + 1 < len(tokens) and tokens[cursor : cursor + 2] == ["#", "["]:
        end = group_end(tokens, cursor + 1, "[", "]")
        attributes.append(tokens[cursor + 2 : end - 1])
        cursor = end
    return attributes, cursor


def attribute_at(tokens: list[str], start: int) -> tuple[list[str], int] | None:
    """Return one outer or inner Rust attribute at a token offset."""
    if tokens[start : start + 2] == ["#", "["]:
        bracket = start + 1
    elif tokens[start : start + 3] == ["#", "!", "["]:
        bracket = start + 2
    else:
        return None
    end = group_end(tokens, bracket, "[", "]")
    return tokens[bracket + 1 : end - 1], end


def derive_path(segment: list[str]) -> tuple[str, ...] | None:
    """Return one simple or qualified derive path."""
    if not segment or not IDENTIFIER.fullmatch(segment[0]):
        return None
    identifiers = [segment[0]]
    cursor = 1
    while cursor < len(segment):
        if (
            cursor + 2 >= len(segment)
            or segment[cursor : cursor + 2] != [":", ":"]
            or not IDENTIFIER.fullmatch(segment[cursor + 2])
        ):
            return None
        identifiers.append(segment[cursor + 2])
        cursor += 3
    return tuple(identifiers)


def check_derive_attribute(path: Path, root: Path, attribute: list[str]) -> bool:
    """Reject a derive macro outside the frozen safe set."""
    if len(attribute) < 4 or attribute[1] != "(" or attribute[-1] != ")":
        report(f"{path.relative_to(root)} has an unsupported derive attribute")
        return False
    valid = True
    for segment in top_level_segments(attribute[2:-1]):
        derive = derive_path(segment)
        allowed = derive in SAFE_QUALIFIED_DERIVES or (
            derive is not None
            and len(derive) == 1
            and derive[0] in SAFE_CONTRACT_DERIVES
        )
        if not allowed:
            name = "::".join(derive) if derive is not None else "unsupported"
            report(f"{path.relative_to(root)} has an unreviewed derive macro {name}")
            valid = False
    return valid


def check_serde_attribute(path: Path, root: Path, attribute: list[str]) -> bool:
    """Reject a simulator-specific serialized name in one Serde attribute."""
    options = {
        token for token in attribute if IDENTIFIER.fullmatch(token)
    } - SAFE_SERDE_OPTIONS
    if options:
        report(
            f"{path.relative_to(root)} has an unreviewed Serde option {min(options)}"
        )
        return False
    for token in attribute:
        literal = rust_string_value(token)
        if literal is None:
            continue
        for fragment in FORBIDDEN_IDENTIFIERS:
            if fragment in normalized(literal):
                report(
                    f"{path.relative_to(root)} has a simulator-specific Serde name {fragment}"
                )
                return False
    return True


def check_contract_attributes(path: Path, root: Path, tokens: list[str]) -> bool:
    """Reject a procedural attribute that can hide a shared contract."""
    valid = True
    cursor = 0
    while cursor < len(tokens):
        parsed = attribute_at(tokens, cursor)
        if parsed is None:
            cursor += 1
            continue
        attribute, cursor = parsed
        name = attribute[0] if attribute else "empty"
        if name not in SAFE_CONTRACT_ATTRIBUTES:
            report(
                f"{path.relative_to(root)} has an unreviewed contract attribute {name}"
            )
            valid = False
        elif name == "derive" and not check_derive_attribute(path, root, attribute):
            valid = False
        elif name == "serde" and not check_serde_attribute(path, root, attribute):
            valid = False
    if not check_protected_macro_imports(path, root, tokens):
        valid = False
    if has_manual_serialization_impl(tokens):
        relative = path.relative_to(root).as_posix()
        try:
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as error:
            report(f"{path.relative_to(root)} cannot be hashed: {error}")
            return False
        if MANUAL_SERIALIZATION_SOURCE_DIGESTS.get(relative) != digest:
            report(f"{path.relative_to(root)} has an unreviewed manual Serde impl")
            valid = False
    return valid


def unqualified_derives(tokens: list[str]) -> set[str]:
    """Return each unqualified derive macro name in one Rust source."""
    derives: set[str] = set()
    cursor = 0
    while cursor < len(tokens):
        parsed = attribute_at(tokens, cursor)
        if parsed is None:
            cursor += 1
            continue
        attribute, cursor = parsed
        if not attribute or attribute[0] != "derive" or len(attribute) < 4:
            continue
        for segment in top_level_segments(attribute[2:-1]):
            derive = derive_path(segment)
            if derive is not None and len(derive) == 1:
                derives.add(derive[0])
    return derives


def check_protected_macro_imports(path: Path, root: Path, tokens: list[str]) -> bool:
    """Bind unqualified Serde and thiserror derives to their canonical crates."""
    required = unqualified_derives(tokens).intersection(
        {"Serialize", "Deserialize", "Error"}
    )
    canonical_imports: set[str] = set()
    valid = True
    cursor = 0
    while cursor < len(tokens):
        if tokens[cursor] != "use":
            cursor += 1
            continue
        try:
            end = tokens.index(";", cursor + 1)
        except ValueError:
            end = len(tokens)
        statement = tokens[cursor + 1 : end]
        root_name = next(
            (token for token in statement if IDENTIFIER.fullmatch(token)), None
        )
        protected = {"Serialize", "Deserialize", "Error"}.intersection(statement)
        if "as" in statement and (protected or root_name in {"serde", "thiserror"}):
            report(f"{path.relative_to(root)} aliases a protected derive import")
            valid = False
        for name in protected.intersection({"Serialize", "Deserialize"}):
            if root_name == "serde":
                canonical_imports.add(name)
            else:
                report(
                    f"{path.relative_to(root)} imports protected derive {name} from a noncanonical crate"
                )
                valid = False
        if "Error" in protected and "Error" in required:
            if root_name == "thiserror":
                canonical_imports.add("Error")
            else:
                report(
                    f"{path.relative_to(root)} imports protected derive Error from a noncanonical crate"
                )
                valid = False
        cursor = end + 1
    for name in sorted(required - canonical_imports):
        report(
            f"{path.relative_to(root)} uses unqualified derive {name} without its canonical import"
        )
        valid = False
    return valid


def has_manual_serialization_impl(tokens: list[str]) -> bool:
    """Return true when source implements Serialize or Deserialize by hand."""
    cursor = 0
    while cursor < len(tokens):
        if tokens[cursor] != "impl":
            cursor += 1
            continue
        end = item_end(tokens, cursor, len(tokens))
        header = tokens[cursor:end]
        body = header.index("{") if "{" in header else len(header)
        before_body = header[:body]
        if "for" in before_body:
            trait = before_body[: before_body.index("for")]
            if "Serialize" in trait or "Deserialize" in trait:
                return True
        if any(
            header[index : index + 2] in (["fn", "serialize"], ["fn", "deserialize"])
            for index in range(len(header) - 1)
        ):
            return True
        cursor = end
    return False


def item_after_visibility(tokens: list[str], start: int) -> int:
    """Return the first token after an optional visibility qualifier."""
    cursor = start
    if cursor < len(tokens) and tokens[cursor] == "pub":
        cursor += 1
        if cursor < len(tokens) and tokens[cursor] == "(":
            cursor = group_end(tokens, cursor, "(", ")")
    return cursor


def macro_invocation_at(
    tokens: list[str], start: int
) -> tuple[str, tuple[str, ...], int] | None:
    """Return one function-like macro invocation at an item start."""
    cursor = start
    if tokens[cursor : cursor + 2] == [":", ":"]:
        cursor += 2
    if cursor >= len(tokens) or not IDENTIFIER.fullmatch(tokens[cursor]):
        return None
    macro_name = tokens[cursor]
    cursor += 1
    while (
        cursor + 2 < len(tokens)
        and tokens[cursor : cursor + 2] == [":", ":"]
        and IDENTIFIER.fullmatch(tokens[cursor + 2])
    ):
        macro_name = tokens[cursor + 2]
        cursor += 3
    if (
        cursor + 1 >= len(tokens)
        or tokens[cursor] != "!"
        or tokens[cursor + 1] not in GROUP_CLOSINGS
    ):
        return None
    opening = tokens[cursor + 1]
    end = group_end(tokens, cursor + 1, opening, GROUP_CLOSINGS[opening])
    return macro_name, tuple(tokens[start:end]), end


def item_end(tokens: list[str], start: int, limit: int) -> int:
    """Return the end of one item without entering its body."""
    cursor = start
    while cursor < limit:
        token = tokens[cursor]
        if token in {"(", "["}:
            cursor = group_end(tokens, cursor, token, GROUP_CLOSINGS[token])
        elif token == "{":
            return group_end(tokens, cursor, "{", "}")
        elif token == ";":
            return cursor + 1
        else:
            cursor += 1
    return limit


def inline_module_body(
    tokens: list[str], start: int, limit: int
) -> tuple[int, int, int] | None:
    """Return the body limits and item end for one inline module."""
    if start + 1 >= limit or tokens[start] != "mod":
        return None
    cursor = start + 2
    while cursor < limit:
        if tokens[cursor] == ";":
            return None
        if tokens[cursor] == "{":
            end = group_end(tokens, cursor, "{", "}")
            return cursor + 1, end - 1, end
        cursor += 1
    return None


def check_item_macros(
    path: Path, root: Path, tokens: list[str]
) -> tuple[bool, frozenset[int]]:
    """Reject macros that can create hidden production contracts."""
    relative = path.relative_to(root).as_posix()
    approved_generated_includes: set[int] = set()

    def check_items(start: int, limit: int, allow_generated_include: bool) -> bool:
        valid = True
        cursor = start
        while cursor < limit:
            attributes, item_start = item_attributes(tokens, cursor)
            item_start = item_after_visibility(tokens, item_start)
            if item_start >= limit:
                break
            if (
                tokens[item_start : item_start + 2] == ["macro_rules", "!"]
                or tokens[item_start] == "macro"
            ):
                report(f"{path.relative_to(root)} has a production macro definition")
                valid = False
            invocation = macro_invocation_at(tokens, item_start)
            if invocation is not None:
                macro_name, invocation_tokens, end = invocation
                allowed = GENERATED_INCLUDE_ALLOWLIST.get(relative)
                if (
                    not allow_generated_include
                    or attributes
                    or allowed != invocation_tokens
                ):
                    report(
                        f"{path.relative_to(root)} has an unreviewed item macro {macro_name}"
                    )
                    valid = False
                else:
                    approved_generated_includes.add(item_start)
                cursor = end
                continue
            module_body = inline_module_body(tokens, item_start, limit)
            if module_body is not None:
                body_start, body_end, end = module_body
                if not check_items(body_start, body_end, False):
                    valid = False
                cursor = end
                continue
            cursor = item_end(tokens, item_start, limit)
        return valid

    return check_items(0, len(tokens), True), frozenset(approved_generated_includes)


def path_is_within(path: Path, root: Path) -> bool:
    """Return true when one resolved path stays below a resolved root."""
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def is_regular_file_without_symlinks(path: Path, root: Path) -> bool:
    """Return true for one regular in-root file with no symlink component."""
    try:
        relative = path.relative_to(root)
    except ValueError:
        return False
    cursor = root
    if cursor.is_symlink():
        return False
    for part in relative.parts:
        cursor /= part
        if cursor.is_symlink():
            return False
    return path.is_file() and path_is_within(path.resolve(), root.resolve())


def check_module_source_boundary(
    path: Path, source_root: Path, root: Path, tokens: list[str]
) -> bool:
    """Reject a production route into an excluded or external Rust source."""
    valid = True
    cursor = 0
    resolved_source_root = source_root.resolve()
    while cursor < len(tokens):
        attributes, item_start = item_attributes(tokens, cursor)
        item_start = item_after_visibility(tokens, item_start)
        if item_start + 1 >= len(tokens) or tokens[item_start] != "mod":
            cursor = max(cursor + 1, item_start)
            continue
        module_name = tokens[item_start + 1]
        requires_test = attributes_require_test(attributes)
        if any(
            attribute and attribute[0] == "cfg_attr" and "path" in attribute
            for attribute in attributes
        ):
            report(f"{path.relative_to(root)} has a conditional module path")
            valid = False
        if module_name in TEST_ONLY_MODULE_NAMES and not requires_test:
            report(
                f"{path.relative_to(root)} module {module_name} is not restricted to cfg(test)"
            )
            valid = False
        attribute_path = path_attribute_value(attributes)
        if any(attribute and attribute[0] == "path" for attribute in attributes) and (
            attribute_path is None
        ):
            report(f"{path.relative_to(root)} has an unsupported module path")
            valid = False
        if attribute_path is not None:
            declared_target = path.parent / attribute_path
            target = declared_target.resolve()
            if not path_is_within(target, resolved_source_root):
                report(
                    f"{path.relative_to(root)} path attribute leaves its source root"
                )
                valid = False
            elif target.suffix != ".rs":
                report(f"{path.relative_to(root)} has a non-Rust module path")
                valid = False
            elif not is_regular_file_without_symlinks(
                declared_target, resolved_source_root
            ):
                report(f"{path.relative_to(root)} has an unsafe module path")
                valid = False
            elif (
                not is_production_path(target, resolved_source_root)
                and not requires_test
            ):
                report(
                    f"{path.relative_to(root)} path attribute imports test-only source without cfg(test)"
                )
                valid = False
        cursor = item_start + 2
    return valid


def check_generated_includes(
    path: Path,
    root: Path,
    tokens: list[str],
    approved_positions: frozenset[int] = frozenset(),
) -> bool:
    """Reject a Rust source include outside the exact frozen baseline."""
    valid = True
    relative = path.relative_to(root).as_posix()
    if relative in GENERATED_INCLUDE_ALLOWLIST and any(
        tokens[index : index + 3] == ["#", "!", "["] for index in range(len(tokens) - 2)
    ):
        report(f"{path.relative_to(root)} has an attributed generated source file")
        valid = False
    cursor = 0
    approved_count = 0
    while cursor + 2 < len(tokens):
        if (
            tokens[cursor : cursor + 2] != ["include", "!"]
            or tokens[cursor + 2] not in GROUP_CLOSINGS
        ):
            cursor += 1
            continue
        opening = tokens[cursor + 2]
        end = group_end(tokens, cursor + 2, opening, GROUP_CLOSINGS[opening])
        invocation = tuple(tokens[cursor:end])
        if (
            GENERATED_INCLUDE_ALLOWLIST.get(relative) != invocation
            or cursor not in approved_positions
        ):
            report(f"{path.relative_to(root)} has an unreviewed generated Rust include")
            valid = False
        else:
            approved_count += 1
            if approved_count > 1:
                report(
                    f"{path.relative_to(root)} repeats its reviewed generated Rust include"
                )
                valid = False
        cursor = end
    if relative in GENERATED_INCLUDE_ALLOWLIST and approved_count == 0:
        report(f"{path.relative_to(root)} omits its reviewed generated Rust include")
        valid = False
    return valid


def first_simulator_fragment(tokens: list[str]) -> str | None:
    """Return the first simulator-specific source fragment."""
    for token in tokens:
        literal_value = rust_string_value(token)
        value = normalized(literal_value if literal_value is not None else token)
        for fragment in FORBIDDEN_IDENTIFIERS:
            if fragment in value:
                return fragment
    return None


def check_simulator_fragments(path: Path, root: Path, tokens: list[str]) -> bool:
    """Bind each shared simulator exception to its exact frozen source."""
    fragment = first_simulator_fragment(tokens)
    if fragment is None:
        return True
    relative = path.relative_to(root).as_posix()
    try:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        report(f"{path.relative_to(root)} cannot be hashed: {error}")
        return False
    if SHARED_SIMULATOR_SOURCE_DIGESTS.get(relative) == digest:
        return True
    report(f"{path.relative_to(root)} has a new simulator-specific token {fragment}")
    return False


def read_controlled_tokens(path: Path, root: Path) -> list[str] | None:
    """Read one Rust source with literals for source-boundary checks."""
    try:
        return rust_tokens(path.read_text(encoding="utf-8"), keep_literals=True)
    except (OSError, UnicodeError, ParseError) as error:
        report(f"{path.relative_to(root)} cannot be parsed: {error}")
        return None


def check_source_controls(path: Path, source_root: Path, root: Path) -> bool:
    """Check module reachability and generated source includes for one file."""
    tokens = read_controlled_tokens(path, root)
    if tokens is None:
        return False
    item_macros_valid, approved_positions = check_item_macros(path, root, tokens)
    return all(
        (
            check_module_source_boundary(path, source_root, root, tokens),
            check_generated_includes(path, root, tokens, approved_positions),
            item_macros_valid,
        )
    )


def production_rust_paths(source_root: Path, root: Path) -> tuple[list[Path], bool]:
    """List production Rust files and reject source-path symlinks."""
    if source_root.is_symlink():
        report(f"{source_root.relative_to(root)} production source root is a symlink")
        return [], False
    if not source_root.is_dir():
        return [], True

    valid = True
    for path in sorted(source_root.rglob("*")):
        if path.is_symlink() and is_production_path(path, source_root):
            report(f"{path.relative_to(root)} production source path is a symlink")
            valid = False
    paths = [
        path
        for path in sorted(source_root.rglob("*.rs"))
        if not path.is_symlink() and is_production_path(path, source_root)
    ]
    for path in paths:
        if not check_source_controls(path, source_root, root):
            valid = False
    return paths, valid


def read_public_types(
    path: Path, root: Path
) -> list[tuple[str, str, list[tuple[str, str]], bool]] | None:
    """Read and parse one Rust source file or report a fail-closed error."""
    try:
        return public_types(rust_tokens(path.read_text(encoding="utf-8")))
    except (OSError, UnicodeError, ParseError) as error:
        report(f"{path.relative_to(root)} cannot be parsed: {error}")
        return None


def check_type_identifiers(
    path: Path,
    root: Path,
    records: list[tuple[str, str, list[tuple[str, str]], bool]],
    allowed: frozenset[tuple[str, str, str]] = frozenset(),
) -> bool:
    """Reject simulator-specific fields and variants in public shared types."""
    valid = True
    relative = path.relative_to(root)
    for _, type_name, identifiers, _ in records:
        for kind, identifier in identifiers:
            if (type_name, kind, identifier) not in allowed and is_forbidden_identifier(
                identifier
            ):
                report(
                    f"{relative} {type_name} has simulator-specific {kind} {identifier}"
                )
                valid = False
    return valid


def check_source_root(source_root: Path, root: Path) -> bool:
    """Check all production Rust files below one shared runtime root."""
    paths, valid = production_rust_paths(source_root, root)
    for path in paths:
        controlled_tokens = read_controlled_tokens(path, root)
        if controlled_tokens is None:
            valid = False
        else:
            if not check_simulator_fragments(path, root, controlled_tokens):
                valid = False
            if not check_contract_attributes(path, root, controlled_tokens):
                valid = False
        records = read_public_types(path, root)
        if records is None:
            valid = False
        elif not check_type_identifiers(path, root, records):
            valid = False
    return valid


def check_campaign_file(path: Path, root: Path) -> bool:
    """Check classified public contracts in one campaign configuration file."""
    controlled_tokens = read_controlled_tokens(path, root)
    records = read_public_types(path, root)
    if controlled_tokens is None or records is None:
        return False
    valid = check_contract_attributes(path, root, controlled_tokens)
    if not check_campaign_aliases(path, root, controlled_tokens):
        valid = False
    relative = path.relative_to(root).as_posix()
    classified_config = relative == "tools/flight-tune-campaign/src/config.rs" or (
        relative.startswith("tools/flight-tune-campaign/src/config/")
    )
    allowed = frozenset(
        (type_name, kind, identifier)
        for allowed_path, type_name, kind, identifier in (
            CAMPAIGN_SHARED_IDENTIFIER_ALLOWLIST
        )
        if allowed_path == relative
    )
    for kind, type_name, identifiers, serialized in records:
        if (relative, type_name) in CAMPAIGN_ADAPTER_TYPE_ALLOWLIST:
            continue
        if classified_config and type_name not in CAMPAIGN_SHARED_TYPES:
            report(
                f"{path.relative_to(root)} has unclassified public campaign contract {type_name}"
            )
            valid = False
        checked_identifiers = (
            identifiers
            if classified_config or serialized
            else [
                (context, identifier)
                for context, identifier in identifiers
                if not context.startswith(
                    ("field_type:", "header_type", "tuple_field_type:", "variant_type:")
                )
            ]
        )
        if not check_type_identifiers(
            path,
            root,
            [(kind, type_name, checked_identifiers, serialized)],
            allowed,
        ):
            valid = False
    return valid


def check_campaign_aliases(path: Path, root: Path, tokens: list[str]) -> bool:
    """Reject a new simulator-specific alias or public re-export."""
    relative = path.relative_to(root).as_posix()
    alias_counts: Counter[tuple[str, str, str]] = Counter()
    public_use_counts: Counter[tuple[str, str]] = Counter()
    valid = True
    cursor = 0
    while cursor < len(tokens):
        if tokens[cursor] == "pub":
            item_start = item_after_visibility(tokens, cursor)
            public_use = item_start < len(tokens) and tokens[item_start] == "use"
            public_extern_crate = tokens[item_start : item_start + 2] == [
                "extern",
                "crate",
            ]
            if public_use or public_extern_crate:
                end = statement_end(tokens, item_start + 1)
                if first_simulator_fragment(tokens[item_start:end]) is not None:
                    public_use_record = (relative, "".join(tokens[item_start:end]))
                    public_use_counts[public_use_record] += 1
                    visibility = "".join(tokens[cursor:item_start])
                    reviewed_public_use = (
                        public_use
                        and (
                            (
                                visibility == "pub"
                                and public_use_record
                                in CAMPAIGN_SIMULATOR_PUBLIC_USE_ALLOWLIST
                            )
                            or (
                                relative,
                                visibility,
                                public_use_record[1],
                            )
                            in CAMPAIGN_SIMULATOR_RESTRICTED_PUBLIC_USE_ALLOWLIST
                        )
                        and public_use_counts[public_use_record] == 1
                    )
                    if not reviewed_public_use:
                        report(
                            f"{path.relative_to(root)} has a simulator-specific public re-export"
                        )
                        valid = False
                cursor = end
                continue
        if tokens[cursor : cursor + 2] == ["extern", "crate"]:
            end = statement_end(tokens, cursor + 2)
            if first_simulator_fragment(tokens[cursor:end]) is not None:
                report(
                    f"{path.relative_to(root)} has a simulator-specific extern crate"
                )
                valid = False
            cursor = end
            continue
        if (
            tokens[cursor] == "type"
            and cursor + 1 < len(tokens)
            and IDENTIFIER.fullmatch(tokens[cursor + 1])
        ):
            end = statement_end(tokens, cursor + 2)
            statement = tokens[cursor + 2 : end - 1]
            if "=" in statement:
                alias_name = tokens[cursor + 1]
                equals = statement.index("=")
                for token in [alias_name, *statement[equals + 1 :]]:
                    if IDENTIFIER.fullmatch(token) and is_forbidden_identifier(token):
                        alias_counts[(relative, alias_name, token)] += 1
            cursor = end
            continue
        if tokens[cursor] == "use":
            end = statement_end(tokens, cursor + 1)
            statement = tokens[cursor + 1 : end - 1]
            if "as" in statement and any(
                IDENTIFIER.fullmatch(token) and is_forbidden_identifier(token)
                for token in statement
            ):
                report(
                    f"{path.relative_to(root)} has a simulator-specific import alias"
                )
                valid = False
            cursor = end
            continue
        cursor += 1
    for record, count in alias_counts.items():
        if count > CAMPAIGN_SIMULATOR_ALIAS_LIMITS.get(record, 0):
            _, alias_name, token = record
            report(
                f"{path.relative_to(root)} has unreviewed simulator type alias {alias_name} for {token}"
            )
            valid = False
    return valid


def check_campaign_contracts(root: Path) -> bool:
    """Check every production contract in the campaign source root."""
    campaign_root = root / "tools/flight-tune-campaign/src"
    paths, valid = production_rust_paths(campaign_root, root)
    for path in paths:
        if not check_campaign_file(path, root):
            valid = False
    return valid


def check_generated_include_inputs(root: Path) -> bool:
    """Check each source file that creates one allowlisted generated include."""
    valid = True
    for include_source, input_paths in GENERATED_INCLUDE_INPUTS.items():
        include_path = root / include_source
        if not include_path.exists() and not any(
            (root / input_path).exists() for input_path in input_paths
        ):
            continue
        package_parts = Path(include_source).parts[:2]
        package_root = root.joinpath(*package_parts)
        for input_path in input_paths:
            generator = root / input_path
            if not is_regular_file_without_symlinks(generator, root):
                if include_path.exists() or generator.exists():
                    report(
                        f"{input_path} generated source input is missing or is a symlink"
                    )
                    valid = False
                continue
            try:
                digest = hashlib.sha256(generator.read_bytes()).hexdigest()
            except OSError as error:
                report(f"{input_path} generated source input cannot be hashed: {error}")
                valid = False
                continue
            if digest not in GENERATED_INPUT_DIGESTS.get(input_path, frozenset()):
                report(f"{input_path} generated source input has an unreviewed digest")
                valid = False
            tokens = read_controlled_tokens(generator, root)
            if tokens is None:
                valid = False
                continue
            if not check_module_source_boundary(generator, package_root, root, tokens):
                valid = False
            if not check_generated_includes(generator, root, tokens):
                valid = False
            if not check_simulator_fragments(generator, root, tokens):
                valid = False
    return valid


def main() -> int:
    """Run all simulator-neutral contract checks."""
    if len(sys.argv) != 2:
        print("usage: check-flight-tune-contracts.py ROOT", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve()
    valid = True
    if not check_cargo_source_overrides(root):
        valid = False
    direct_dependency_allowlist = read_direct_dependency_allowlist(root)
    if direct_dependency_allowlist is None:
        return 1
    registry_names: set[str] = set()
    for manifest, allowed_adapter_dependencies in (
        (root / "crates/pilotage-trial/Cargo.toml", frozenset()),
        (root / "crates/pilotage-tuning-feedback/Cargo.toml", frozenset()),
        (root / "tools/flight-tune/Cargo.toml", frozenset()),
        (
            root / "tools/flight-tune-aviate/Cargo.toml",
            frozenset(),
        ),
        (
            root / "tools/flight-tune-campaign/Cargo.toml",
            frozenset(FORBIDDEN_PACKAGES),
        ),
    ):
        manifest_valid, manifest_registry_names = check_manifest(
            manifest, root, direct_dependency_allowlist, allowed_adapter_dependencies
        )
        registry_names.update(manifest_registry_names)
        if not manifest_valid:
            valid = False
    if not check_cargo_lock_packages(root, registry_names):
        valid = False
    for source_root in (
        root / "crates/pilotage-trial/src",
        root / "tools/flight-tune/src",
    ):
        if not check_source_root(source_root, root):
            valid = False
    aviate_paths, aviate_sources_valid = production_rust_paths(
        root / "tools/flight-tune-aviate/src", root
    )
    if not aviate_sources_valid:
        valid = False
    for path in aviate_paths:
        controlled_tokens = read_controlled_tokens(path, root)
        if controlled_tokens is None or not check_contract_attributes(
            path, root, controlled_tokens
        ):
            valid = False
    if not check_campaign_contracts(root):
        valid = False
    if not check_generated_include_inputs(root):
        valid = False
    return 0 if valid else 1


if __name__ == "__main__":
    raise SystemExit(main())
