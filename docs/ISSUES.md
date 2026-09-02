# Known TDA schema gaps

The TDA contracts below are relevant to the localhost editor service or the AI
service, but they are intentionally not transcribed into `schema/tda` yet.
Xenomorph must reproduce their TypeScript semantics before their source
TypeScript declarations can be removed.

## `unknown`, `object`, and recursive JSON

The current TypeScript target has no native mapping for `unknown` or `object`.
The built-in `json` type maps to `string`, so it is not equivalent to TDA's
recursive `JsonValue`.

Blocked contracts include:

- `Requirement`, `CustomAttribute`, and `SimilarRequirement` in TDA `env.d.ts`.
  Their dynamic value unions contain `Record<string, unknown>`.
- `JsonPrimitive` and `JsonValue` in TDA `src/model/json-types.ts`.
- `RequirementSearchRequirement.linked_designs`, `SynthStreamEvent.meta`,
  `StreamEvent.meta`, and responses that transitively contain those fields.
- `ExtractSignalsResponse.stats`, `GeneralNotesRes.stats`, and
  `TestGenerationWorkflowStreamPayload.generation_input` in TDA
  `src/api/prompts/ai.model.forms.ts`.
- `ModuleEntryMergedOut` and `ModuleEntriesMergedOut` in TDA
  `src/api/prompts/dngGetData.types.ts`. Merged rows intentionally allow
  arbitrary `unknown`-valued custom attributes.
- `TestSpecLookupData.test_spec_procedure` and
  `PurposeSearchTestSpec.test_spec_procedure`.
- `GenerateTestSpecsOptions.current_design`, which accepts `string | object`.
- `TimelineEntry.input` and `TimelineEntry.output` in TDA `src/model/ui.ts`.

Required work: provide target mappings for `unknown` and `object`, and a
recursive JSON value/object representation that remains JSON rather than a
serialized string.

## Multipart browser types

`File`, `FormData`, and the `TypedFormData<T>` brand describe browser-side
multipart construction rather than the JSON body seen by the backend.
Xenomorph cannot currently import or model these DOM types.

Affected declarations in TDA `src/api/prompts/ai.model.forms.ts`:

- `RequirementClusterStreamReq`
- `FeedbackForm`
- `ExtractSignalsReq`
- `GenerateGeneralNotesReq`
- `ExtractPicturesReq`
- `TestGenerationWorkflowStreamPayload`
- `TypedFormData`
- `AIAPIFormEndpoints`

Required work: define a first-class multipart/file descriptor or a
TypeScript-target escape hatch that preserves the DOM types while allowing
other targets to describe the received byte/file parts.

## Endpoint maps and TypeScript metaprogramming

The endpoint registries and client helpers use mapped, conditional, indexed,
and function types that are outside the schema language's current data-shape
scope:

- `Call`, `AIApi`, `AiApiNonFormDataKeys`, and `AiApiFormDataKeys` in TDA
  `src/api/prompts`.
- `ExactTestCase<TT>` and `TestCaseMap` in TDA `env.d.ts`.
- `GenerateTestSpecsOptions` because it intentionally accepts normalized and
  pre-normalized union shapes, including `Partial<T>`.

These should stay as handwritten client integration types unless Xenomorph
adds an endpoint/RPC model and the required TypeScript type operations.

## External response types

The localhost folder-structure endpoints return PrimeVue `TreeNode`. Xenomorph
cannot import declarations from `primevue/treenode`, and copying a local
structural subset would not guarantee semantic identity with future PrimeVue
versions.

Affected endpoints in TDA `src/api/apiCalls.ts`:

- `/lab-test-template/get-folder-structure/`
- `/lab-test-template/get-complete-folder-structure/`

Required work: either define and own a backend folder-tree DTO or support
explicit target-native external type imports.

## Blocked transitive editor contracts

The following otherwise ordinary editor contracts remain blocked because they
contain one of the non-reproducible types above:

- `TestSpecification` (`Requirement[]`)
- `DesignScenario` and `TestDesign` because `DesignScenario` contains
  `TestSpecification[]`.
- `DesignStateAndReqsForScenarioRes` (`Requirement[]`)
- `LinkedScenarioRes` (`Requirement[]`)
- Full `LabTestTemplate`, `ReleaseTestTemplate`, and `VehicleTestTemplate`
  graphs because their shared `General` object contains `Requirement[]`.

Do not weaken these fields to `any`, `string`, `Map`, or an incomplete explicit
struct merely to make generation pass; each substitution changes the public
contract.
