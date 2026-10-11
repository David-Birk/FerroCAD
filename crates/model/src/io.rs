//! `.fcad` files: the document as readable, diff-friendly JSON.
//!
//! Only the recipe is stored (parameters, sketches, features) – solids are
//! rebuilt on load, like Fusion's history-based files. Every reference is
//! validated so a damaged file gives an error message, never a panic.

use crate::document::Document;
use crate::feature::{
    ExtrudeFeature, Feature, FeatureId, FeatureKind, FilletFeature, Operation, RevolveAxis,
    RevolveFeature, SketchFeature,
};
use crate::geom::Plane;
use crate::json::{self, obj, Json};
use crate::kernel::EdgeRef;
use crate::params::ParamTable;
use ferrocad_sketch::{
    Constraint, ConstraintId, Entity, EntityData, EntityId, ParamId, PointId, Sketch, SketchData,
};

pub const FORMAT: &str = "ferrocad";
pub const VERSION: usize = 1;
pub const EXTENSION: &str = "fcad";

#[derive(Clone, Debug, PartialEq)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

impl From<json::JsonError> for LoadError {
    fn from(e: json::JsonError) -> Self {
        LoadError(e.to_string())
    }
}

impl From<String> for LoadError {
    fn from(e: String) -> Self {
        LoadError(e)
    }
}

type R<T> = Result<T, LoadError>;

// ==================================================================== save

pub fn save(doc: &Document) -> String {
    let params: Vec<Json> = doc
        .params
        .iter()
        .map(|p| {
            obj([
                ("name", p.name.as_str().into()),
                ("expression", p.expression.as_str().into()),
                ("comment", p.comment.as_str().into()),
            ])
        })
        .collect();
    obj([
        ("format", FORMAT.into()),
        ("version", VERSION.into()),
        ("params", Json::Arr(params)),
        ("marker", doc.marker.into()),
        ("next_id", doc.next_id.into()),
        ("features", Json::Arr(doc.features.iter().map(feature_to_json).collect())),
    ])
    .to_pretty()
}

fn op_name(op: Operation) -> &'static str {
    match op {
        Operation::NewBody => "new_body",
        Operation::Join => "join",
        Operation::Cut => "cut",
        Operation::Intersect => "intersect",
    }
}

fn plane_to_json(p: &Plane) -> Json {
    obj([("origin", p.origin.into()), ("x_axis", p.x_axis.into()), ("y_axis", p.y_axis.into())])
}

fn feature_to_json(f: &Feature) -> Json {
    let mut fields: Vec<(String, Json)> = vec![
        ("id".into(), f.id.0.into()),
        ("name".into(), f.name.as_str().into()),
        ("suppressed".into(), f.suppressed.into()),
    ];
    let mut add = |k: &str, v: Json| fields.push((k.to_string(), v));
    match &f.kind {
        FeatureKind::Sketch(s) => {
            add("type", "sketch".into());
            add("plane", plane_to_json(&s.plane));
            add(
                "driven",
                Json::Arr(
                    s.driven
                        .iter()
                        .map(|(c, e)| obj([("constraint", c.0.into()), ("expression", e.as_str().into())]))
                        .collect(),
                ),
            );
            add("sketch", sketch_to_json(&s.sketch));
        }
        FeatureKind::Extrude(e) => {
            add("type", "extrude".into());
            add("sketch", e.sketch.0.into());
            add("profile_point", e.profile_point.into());
            add("distance", e.distance.as_str().into());
            add("symmetric", e.symmetric.into());
            add("operation", op_name(e.operation).into());
        }
        FeatureKind::Revolve(r) => {
            add("type", "revolve".into());
            add("sketch", r.sketch.0.into());
            add("profile_point", r.profile_point.into());
            add(
                "axis",
                match r.axis {
                    RevolveAxis::SketchX => "x".into(),
                    RevolveAxis::SketchY => "y".into(),
                    RevolveAxis::Line(l) => obj([("line", l.0.into())]),
                },
            );
            add("angle", r.angle.as_str().into());
            add("operation", op_name(r.operation).into());
        }
        FeatureKind::Fillet(fl) => {
            add("type", "fillet".into());
            add("edges", Json::Arr(fl.edges.iter().map(|e| e.point.into()).collect()));
            add("radius", fl.radius.as_str().into());
            add("chamfer", fl.chamfer.into());
        }
    }
    Json::Obj(fields)
}

fn constraint_type(c: &Constraint) -> &'static str {
    use Constraint::*;
    match c {
        Coincident(..) => "coincident",
        Horizontal(..) => "horizontal",
        Vertical(..) => "vertical",
        Parallel(..) => "parallel",
        Perpendicular(..) => "perpendicular",
        Equal(..) => "equal",
        PointOnLine(..) => "point_on_line",
        PointOnCircle(..) => "point_on_circle",
        Midpoint(..) => "midpoint",
        Tangent(..) => "tangent",
        Concentric(..) => "concentric",
        Distance(..) => "distance",
        Length(..) => "length",
        HorizontalDistance(..) => "horizontal_distance",
        VerticalDistance(..) => "vertical_distance",
        Radius(..) => "radius",
        Diameter(..) => "diameter",
        Angle(..) => "angle",
    }
}

fn sketch_to_json(sk: &Sketch) -> Json {
    let d = sk.to_data();
    let entities = d
        .entities
        .iter()
        .map(|e| {
            let mut f: Vec<(String, Json)> = match e.entity {
                Entity::Line { a, b } => {
                    vec![("type".into(), "line".into()), ("a".into(), a.0.into()), ("b".into(), b.0.into())]
                }
                Entity::Circle { center, radius } => vec![
                    ("type".into(), "circle".into()),
                    ("center".into(), center.0.into()),
                    ("radius".into(), radius.0.into()),
                ],
                Entity::Arc { center, start, end } => vec![
                    ("type".into(), "arc".into()),
                    ("center".into(), center.0.into()),
                    ("start".into(), start.0.into()),
                    ("end".into(), end.0.into()),
                ],
            };
            f.push(("construction".into(), e.construction.into()));
            f.push(("alive".into(), e.alive.into()));
            Json::Obj(f)
        })
        .collect();
    let constraints = d
        .constraints
        .iter()
        .map(|c| match c {
            None => Json::Null,
            Some(c) => {
                let mut f: Vec<(String, Json)> = vec![("type".into(), constraint_type(c).into())];
                let pts = c.points();
                if !pts.is_empty() {
                    f.push(("points".into(), pts.iter().map(|p| p.0).collect::<Vec<_>>().into()));
                }
                let ents = c.entities();
                if !ents.is_empty() {
                    f.push(("entities".into(), ents.iter().map(|e| e.0).collect::<Vec<_>>().into()));
                }
                if let Some(v) = c.value() {
                    f.push(("value".into(), v.into()));
                }
                Json::Obj(f)
            }
        })
        .collect();
    obj([
        ("params", d.params.into()),
        ("fixed", d.fixed.into()),
        ("points", Json::Arr(d.points.iter().map(|(x, y)| vec![*x, *y].into()).collect())),
        ("entities", Json::Arr(entities)),
        ("constraints", Json::Arr(constraints)),
    ])
}

// ==================================================================== load

pub fn load(text: &str) -> R<Document> {
    let root = json::parse(text)?;
    if root.get("format")?.as_str()? != FORMAT {
        return Err(LoadError("not a ferrocad file".into()));
    }
    let version = root.get("version")?.as_usize()?;
    if version > VERSION {
        return Err(LoadError(format!(
            "file was written by a newer ferrocad (format {version}, this build reads {VERSION})"
        )));
    }

    let mut params = ParamTable::new();
    // Parameters may reference each other in any order, so insert all of
    // them without evaluation first, then validate.
    let mut raw = Vec::new();
    for p in root.get("params")?.as_arr()? {
        let name = p.get("name")?.as_str()?.to_string();
        let expr = p.get("expression")?.as_str()?.to_string();
        let comment = p.get_opt("comment").map(|c| c.as_str().map(str::to_string)).transpose()?;
        raw.push((name, expr, comment.unwrap_or_default()));
    }
    params.set_unchecked(raw.iter().map(|(n, e, c)| (n.clone(), e.clone(), c.clone())).collect())?;
    for (name, _, _) in &raw {
        params.value(name).map_err(|e| LoadError(format!("parameter '{name}': {e}")))?;
    }

    let mut features = Vec::new();
    for f in root.get("features")?.as_arr()? {
        let name = f.get_opt("name").and_then(|n| n.as_str().ok()).unwrap_or("?").to_string();
        let feature = feature_from_json(f).map_err(|e| LoadError(format!("feature '{name}': {e}")))?;
        features.push(feature);
    }

    let mut ids: Vec<u64> = features.iter().map(|f: &Feature| f.id.0).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(LoadError("duplicate feature ids".into()));
    }
    let marker = root.get("marker")?.as_usize()?;
    if marker > features.len() {
        return Err(LoadError("timeline marker is past the end".into()));
    }
    let next_id = (root.get("next_id")?.as_usize()? as u64).max(ids.last().copied().unwrap_or(0));

    Ok(Document { params, features, marker, next_id })
}

fn op_from(j: &Json) -> R<Operation> {
    Ok(match j.as_str()? {
        "new_body" => Operation::NewBody,
        "join" => Operation::Join,
        "cut" => Operation::Cut,
        "intersect" => Operation::Intersect,
        other => return Err(LoadError(format!("unknown operation '{other}'"))),
    })
}

fn feature_from_json(f: &Json) -> R<Feature> {
    let id = FeatureId(f.get("id")?.as_usize()? as u64);
    let name = f.get("name")?.as_str()?.to_string();
    let suppressed = f.get("suppressed")?.as_bool()?;
    let kind = match f.get("type")?.as_str()? {
        "sketch" => {
            let p = f.get("plane")?;
            let plane = Plane {
                origin: p.get("origin")?.as_vec3()?,
                x_axis: p.get("x_axis")?.as_vec3()?,
                y_axis: p.get("y_axis")?.as_vec3()?,
            };
            let sketch = sketch_from_json(f.get("sketch")?)?;
            let mut driven = Vec::new();
            for d in f.get("driven")?.as_arr()? {
                let c = ConstraintId(d.get("constraint")?.as_usize()?);
                if sketch.constraint(c).is_none_or(|c| !c.is_dimension()) {
                    return Err(LoadError("driven value points at a missing dimension".into()));
                }
                driven.push((c, d.get("expression")?.as_str()?.to_string()));
            }
            FeatureKind::Sketch(SketchFeature { plane, sketch, driven })
        }
        "extrude" => FeatureKind::Extrude(ExtrudeFeature {
            sketch: FeatureId(f.get("sketch")?.as_usize()? as u64),
            profile_point: f.get("profile_point")?.as_vec2()?,
            distance: f.get("distance")?.as_str()?.to_string(),
            symmetric: f.get("symmetric")?.as_bool()?,
            operation: op_from(f.get("operation")?)?,
        }),
        "revolve" => {
            let axis = match f.get("axis")? {
                Json::Str(s) if s == "x" => RevolveAxis::SketchX,
                Json::Str(s) if s == "y" => RevolveAxis::SketchY,
                a => RevolveAxis::Line(EntityId(a.get("line")?.as_usize()?)),
            };
            FeatureKind::Revolve(RevolveFeature {
                sketch: FeatureId(f.get("sketch")?.as_usize()? as u64),
                profile_point: f.get("profile_point")?.as_vec2()?,
                axis,
                angle: f.get("angle")?.as_str()?.to_string(),
                operation: op_from(f.get("operation")?)?,
            })
        }
        "fillet" => FeatureKind::Fillet(FilletFeature {
            edges: f
                .get("edges")?
                .as_arr()?
                .iter()
                .map(|e| e.as_vec3().map(|point| EdgeRef { point }))
                .collect::<Result<_, _>>()?,
            radius: f.get("radius")?.as_str()?.to_string(),
            chamfer: f.get("chamfer")?.as_bool()?,
        }),
        other => return Err(LoadError(format!("unknown feature type '{other}'"))),
    };
    Ok(Feature { id, name, suppressed, kind })
}

fn sketch_from_json(j: &Json) -> R<Sketch> {
    let params = j.get("params")?.as_arr()?.iter().map(Json::as_f64).collect::<Result<Vec<_>, _>>()?;
    let fixed = j.get("fixed")?.as_arr()?.iter().map(Json::as_bool).collect::<Result<Vec<_>, _>>()?;
    let points = j
        .get("points")?
        .as_arr()?
        .iter()
        .map(|p| match p.as_arr()? {
            [x, y] => Ok((x.as_usize()?, y.as_usize()?)),
            _ => Err(LoadError("point must be [x_param, y_param]".into())),
        })
        .collect::<R<Vec<_>>>()?;
    let pid = |j: &Json, k: &str| -> R<PointId> { Ok(PointId(j.get(k)?.as_usize()?)) };
    let entities = j
        .get("entities")?
        .as_arr()?
        .iter()
        .map(|e| {
            let entity = match e.get("type")?.as_str()? {
                "line" => Entity::Line { a: pid(e, "a")?, b: pid(e, "b")? },
                "circle" => Entity::Circle {
                    center: pid(e, "center")?,
                    radius: ParamId(e.get("radius")?.as_usize()?),
                },
                "arc" => Entity::Arc { center: pid(e, "center")?, start: pid(e, "start")?, end: pid(e, "end")? },
                other => return Err(LoadError(format!("unknown entity type '{other}'"))),
            };
            Ok(EntityData {
                entity,
                construction: e.get("construction")?.as_bool()?,
                alive: e.get("alive")?.as_bool()?,
            })
        })
        .collect::<R<Vec<_>>>()?;
    let constraints = j
        .get("constraints")?
        .as_arr()?
        .iter()
        .map(|c| match c {
            Json::Null => Ok(None),
            c => constraint_from_json(c).map(Some),
        })
        .collect::<R<Vec<_>>>()?;
    Ok(Sketch::from_data(SketchData { params, fixed, points, entities, constraints })?)
}

fn constraint_from_json(c: &Json) -> R<Constraint> {
    let ids = |key: &str| -> R<Vec<usize>> {
        match c.get_opt(key) {
            None => Ok(vec![]),
            Some(a) => Ok(a.as_arr()?.iter().map(Json::as_usize).collect::<Result<_, _>>()?),
        }
    };
    let p: Vec<PointId> = ids("points")?.into_iter().map(PointId).collect();
    let e: Vec<EntityId> = ids("entities")?.into_iter().map(EntityId).collect();
    let v = c.get_opt("value").map(Json::as_f64).transpose()?;
    let t = c.get("type")?.as_str()?;
    let shape = (p.len(), e.len(), v.is_some());

    use Constraint::*;
    let made = match (t, shape) {
        ("coincident", (2, 0, false)) => Coincident(p[0], p[1]),
        ("horizontal", (0, 1, false)) => Horizontal(e[0]),
        ("vertical", (0, 1, false)) => Vertical(e[0]),
        ("parallel", (0, 2, false)) => Parallel(e[0], e[1]),
        ("perpendicular", (0, 2, false)) => Perpendicular(e[0], e[1]),
        ("equal", (0, 2, false)) => Equal(e[0], e[1]),
        ("point_on_line", (1, 1, false)) => PointOnLine(p[0], e[0]),
        ("point_on_circle", (1, 1, false)) => PointOnCircle(p[0], e[0]),
        ("midpoint", (1, 1, false)) => Midpoint(p[0], e[0]),
        ("tangent", (0, 2, false)) => Tangent(e[0], e[1]),
        ("concentric", (0, 2, false)) => Concentric(e[0], e[1]),
        ("distance", (2, 0, true)) => Distance(p[0], p[1], v.unwrap()),
        ("length", (0, 1, true)) => Length(e[0], v.unwrap()),
        ("horizontal_distance", (2, 0, true)) => HorizontalDistance(p[0], p[1], v.unwrap()),
        ("vertical_distance", (2, 0, true)) => VerticalDistance(p[0], p[1], v.unwrap()),
        ("radius", (0, 1, true)) => Radius(e[0], v.unwrap()),
        ("diameter", (0, 1, true)) => Diameter(e[0], v.unwrap()),
        ("angle", (0, 2, true)) => Angle(e[0], e[1], v.unwrap()),
        _ => return Err(LoadError(format!("malformed '{t}' constraint"))),
    };
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_constraint_type_roundtrips() {
        let mut sk = Sketch::new();
        let l1 = sk.add_line([0.0, 0.0], [10.0, 0.0]);
        let l2 = sk.add_line([0.0, 5.0], [10.0, 6.0]);
        let c = sk.add_circle([5.0, 20.0], 3.0);
        let (a, b) = match *sk.entity(l1) {
            Entity::Line { a, b } => (a, b),
            _ => unreachable!(),
        };
        use Constraint::*;
        let all = vec![
            Coincident(a, b), Horizontal(l1), Vertical(l1), Parallel(l1, l2), Perpendicular(l1, l2),
            Equal(l1, l2), PointOnLine(a, l2), PointOnCircle(a, c), Midpoint(a, l1), Tangent(l1, c),
            Concentric(c, c), Distance(a, b, 1.5), Length(l1, 0.1 + 0.2), HorizontalDistance(a, b, -2.0),
            VerticalDistance(a, b, 3.0), Radius(c, 4.0), Diameter(c, 8.0), Angle(l1, l2, 0.7),
        ];
        for k in &all {
            sk.constrain(k.clone()); // not solved – only the data matters here
        }
        let back = sketch_from_json(&sketch_to_json(&sk)).unwrap();
        assert_eq!(back.to_data(), sk.to_data());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(load("not json").is_err());
        assert!(load(r#"{"format":"other","version":1}"#).unwrap_err().0.contains("not a ferrocad"));
        let future = r#"{"format":"ferrocad","version":99,"params":[],"marker":0,"next_id":0,"features":[]}"#;
        assert!(load(future).unwrap_err().0.contains("newer"));
        let bad_marker = r#"{"format":"ferrocad","version":1,"params":[],"marker":3,"next_id":0,"features":[]}"#;
        assert!(load(bad_marker).is_err());
        let cycle = r#"{"format":"ferrocad","version":1,"params":[
            {"name":"a","expression":"b"},{"name":"b","expression":"a"}],
            "marker":0,"next_id":0,"features":[]}"#;
        assert!(load(cycle).unwrap_err().0.contains("parameter"));
    }

    #[test]
    fn parameters_may_reference_later_ones() {
        let text = r#"{"format":"ferrocad","version":1,"params":[
            {"name":"a","expression":"b * 2"},{"name":"b","expression":"3"}],
            "marker":0,"next_id":0,"features":[]}"#;
        let doc = load(text).unwrap();
        assert_eq!(doc.params.value("a").unwrap(), 6.0);
        // order is preserved on save
        let again = load(&save(&doc)).unwrap();
        let names: Vec<_> = again.params.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, ["a", "b"]);
    }
}
