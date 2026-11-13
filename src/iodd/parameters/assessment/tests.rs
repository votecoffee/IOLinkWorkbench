use super::*;
use crate::{iodd, profile::Profile, transport};

const XML: &str = include_str!("../../../../samples/sensor-values.xml");

fn profile(xml: &str) -> Result<Profile> {
    let catalog = iodd::parse(xml, "original synthetic value definitions".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn boolean_and_integer_aliases_match_without_losing_exact_wide_values() -> Result<()> {
    let profile = profile(XML)?;
    let boolean = profile.parameter("V_SIM_Enabled")?;
    let enabled = boolean.decode(&[1]);
    assert_eq!(enabled.raw_value.as_deref(), Some("true"));
    assert_eq!(enabled.choice_name.as_deref(), Some("Enabled"));
    assert_eq!(
        enabled.value_assessment.unwrap().status,
        AssessmentStatus::Allowed
    );
    assert_eq!(
        boolean.decode(&[0]).choice_name.as_deref(),
        Some("Disabled")
    );
    let wide = profile.parameter("V_SIM_Wide")?;
    let maximum = wide.decode(&u64::MAX.to_be_bytes());
    assert_eq!(maximum.raw_value.as_deref(), Some("18446744073709551615"));
    assert_eq!(
        maximum.choice_name.as_deref(),
        Some("Maximum unsigned value")
    );
    for (number, expected) in [
        (9007199254740993u64, AssessmentStatus::Allowed),
        (9007199254740994, AssessmentStatus::OutsideDefinition),
    ] {
        let value = wide.decode(&number.to_be_bytes());
        assert_eq!(value.value_assessment.unwrap().status, expected);
        assert!(value.decode_error.is_none());
    }
    Ok(())
}

#[test]
fn float_comparison_uses_binary32_semantics_and_inclusive_ranges() -> Result<()> {
    let profile = profile(XML)?;
    let parameter = profile.parameter("V_SIM_Float")?;
    let nominal = parameter.decode(&1.25f32.to_be_bytes());
    assert_eq!(nominal.choice_name.as_deref(), Some("Nominal float"));
    assert_eq!(nominal.displays[0].value, "12.50");
    for (number, status) in [
        (-1.0f32, AssessmentStatus::Allowed),
        (2.5, AssessmentStatus::Allowed),
        (f32::INFINITY, AssessmentStatus::Allowed),
        (f32::NEG_INFINITY, AssessmentStatus::Allowed),
        (1.5, AssessmentStatus::OutsideDefinition),
        (f32::NAN, AssessmentStatus::NotComparable),
    ] {
        let value = parameter.decode(&number.to_be_bytes());
        assert_eq!(value.value_assessment.unwrap().status, status);
    }
    let mut parameter = parameter.clone();
    parameter.ranges.clear();
    parameter.choices[0].value = "0.10000000149011612".into();
    assert_eq!(
        parameter
            .decode(&0.1f32.to_be_bytes())
            .choice_name
            .as_deref(),
        Some("Nominal float")
    );
    parameter.choices[0].value = "-0.0".into();
    assert!(
        parameter
            .decode(&0.0f32.to_be_bytes())
            .choice_name
            .is_some()
    );
    Ok(())
}

#[test]
fn whole_member_array_and_snapshot_views_keep_assessments_separate_from_scaling() -> Result<()> {
    let profile = Profile::parse(&toml::to_string_pretty(&profile(XML)?)?)?;
    for (id, key) in [("V_SIM_ValueRecord", true), ("V_SIM_ValueArray", false)] {
        let whole = transport::read_parameter(&profile, id)?;
        let items = if key {
            whole.record_items
        } else {
            whole.array_items
        };
        assert_eq!(items.len(), 3);
        for item in items {
            let member = transport::read_parameter_at(&profile, id, item.subindex)?;
            assert_eq!(
                toml::to_string(&item.value.value_assessment)?,
                toml::to_string(&member.value_assessment)?
            );
            assert_eq!(item.value.choice_name, member.choice_name);
        }
    }
    let value = transport::read_parameter_at(&profile, "V_SIM_ValueRecord", 2)?;
    assert_eq!(value.raw_value.as_deref(), Some("-5"));
    assert_eq!(value.choice_name.as_deref(), Some("Negative offset"));
    assert_eq!(value.displays[0].value, "-10");
    let (snapshot, _) = crate::parameter_snapshot::read(&profile, "V_SIM_Wide", 0)?;
    let review = snapshot.review()?;
    assert_eq!(
        review.decoded.value_assessment.unwrap().status,
        AssessmentStatus::Allowed
    );
    Ok(())
}

#[test]
fn invalid_ambiguous_and_overlapping_definitions_are_reported_without_guessing() -> Result<()> {
    let profile = profile(XML)?;
    let boolean = profile.parameter("V_SIM_Enabled")?;
    let mut ambiguous = boolean.clone();
    ambiguous.choices.push(super::super::Choice {
        value: "true".into(),
        name: "Alias".into(),
    });
    let value = ambiguous.decode(&[1]);
    assert!(value.choice_name.is_none() && value.decode_error.is_none());
    assert_eq!(
        value.value_assessment.unwrap().status,
        AssessmentStatus::DefinitionNeedsReview
    );
    let mut unsigned = profile.parameter("V_SIM_Wide")?.clone();
    unsigned.bits = Some(8);
    for (lower, upper) in [("4", "4"), ("5", "4"), ("-1", "4"), ("0", "256")] {
        unsigned.choices.clear();
        unsigned.ranges = vec![super::super::ValueRange {
            name: None,
            lower: lower.into(),
            upper: upper.into(),
        }];
        assert_eq!(
            unsigned.decode(&[3]).value_assessment.unwrap().status,
            AssessmentStatus::DefinitionNeedsReview
        );
    }
    unsigned.ranges = vec![
        super::super::ValueRange {
            name: None,
            lower: "0".into(),
            upper: "4".into(),
        },
        super::super::ValueRange {
            name: None,
            lower: "4".into(),
            upper: "6".into(),
        },
    ];
    assert_eq!(
        unsigned.decode(&[4]).value_assessment.unwrap().status,
        AssessmentStatus::DefinitionNeedsReview
    );
    let mut float = profile.parameter("V_SIM_Float")?.clone();
    for value in ["NaN", "inf", "1e999", "0x3F800000", "1e", ".", "1e2e3"] {
        float.choices[0].value = value.into();
        assert_eq!(
            float
                .decode(&1.25f32.to_be_bytes())
                .value_assessment
                .unwrap()
                .status,
            AssessmentStatus::DefinitionNeedsReview
        );
    }
    Ok(())
}

#[test]
fn unrestricted_unsupported_and_malformed_payloads_have_no_false_allowed_claim() -> Result<()> {
    let mut parameter = profile(XML)?.parameter("V_SIM_Wide")?.clone();
    parameter.choices.clear();
    parameter.ranges.clear();
    assert!(
        parameter
            .decode(&0u64.to_be_bytes())
            .value_assessment
            .is_none()
    );
    parameter.datatype = "StringT".into();
    parameter.fixed_length = Some(8);
    parameter.encoding = Some("UTF-8".into());
    parameter.choices.push(super::super::Choice {
        value: "OK".into(),
        name: "Text".into(),
    });
    let text = parameter.decode(b"OK");
    assert_eq!(
        text.value_assessment.unwrap().status,
        AssessmentStatus::NotEvaluated
    );
    assert!(text.choice_name.is_none());
    assert!(parameter.decode(&[0xFF]).value_assessment.is_none());
    Ok(())
}

#[test]
fn standard_choice_references_compare_typed_values_and_reject_alias_redefinition() -> Result<()> {
    let definition = profile(XML)?.parameter("V_SIM_Enabled")?.clone();
    let xml = r#"<StdVariableRef xmlns="http://www.io-link.com/IODD/2010/10"><StdSingleValueRef value="true"/></StdVariableRef>"#;
    let document = roxmltree::Document::parse(xml)?;
    let resolved =
        super::super::standard::resolve(&definition, document.root_element(), &Default::default())?;
    assert_eq!(resolved.choices.len(), 1);
    assert_eq!(
        resolved.decode(&[1]).choice_name.as_deref(),
        Some("Enabled")
    );
    let replacement = xml.replace("StdSingleValueRef", "SingleValue");
    let document = roxmltree::Document::parse(&replacement)?;
    assert!(
        super::super::standard::resolve(&definition, document.root_element(), &Default::default())
            .is_err()
    );
    let mut duplicate = definition.clone();
    duplicate.choices.push(super::super::Choice {
        value: "true".into(),
        name: "Alias".into(),
    });
    let document = roxmltree::Document::parse(xml)?;
    assert!(
        super::super::standard::resolve(&duplicate, document.root_element(), &Default::default())
            .is_err()
    );
    Ok(())
}

#[test]
fn signed_extremes_whitespace_and_definition_diagnostics_remain_bounded() -> Result<()> {
    let mut parameter = profile(XML)?.parameter("V_SIM_Wide")?.clone();
    parameter.datatype = "IntegerT".into();
    parameter.choices = vec![super::super::Choice {
        value: " \t-09223372036854775808\n".into(),
        name: "Signed minimum".into(),
    }];
    parameter.ranges = vec![super::super::ValueRange {
        name: None,
        lower: "+000".into(),
        upper: "+9223372036854775807".into(),
    }];
    let minimum = parameter.decode(&i64::MIN.to_be_bytes());
    assert_eq!(minimum.choice_name.as_deref(), Some("Signed minimum"));
    assert_eq!(
        minimum.value_assessment.unwrap().status,
        AssessmentStatus::Allowed
    );
    assert_eq!(
        parameter
            .decode(&i64::MAX.to_be_bytes())
            .value_assessment
            .unwrap()
            .status,
        AssessmentStatus::Allowed
    );
    assert_eq!(
        parameter
            .decode(&(-1i64).to_be_bytes())
            .value_assessment
            .unwrap()
            .status,
        AssessmentStatus::OutsideDefinition
    );
    parameter.choices = (0..100)
        .map(|_| super::super::Choice {
            value: "9223372036854775808".into(),
            name: "Out of domain".into(),
        })
        .collect();
    let assessment = parameter
        .decode(&0i64.to_be_bytes())
        .value_assessment
        .unwrap();
    assert_eq!(assessment.status, AssessmentStatus::DefinitionNeedsReview);
    assert_eq!(assessment.notes.len(), 17);
    assert!(assessment.notes.last().unwrap().contains("omitted"));
    Ok(())
}
