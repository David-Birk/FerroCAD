//! `.fcad` files: the document as readable, diff-friendly JSON.
//!
//! Only the recipe is stored (parameters, sketches, features) – solids are
//! rebuilt on load, like Fusion's history-based files. Every reference is
//! validated so a damaged file gives an error message, never a panic.

use crate::document::Document;
use crate::feature::*;
use crate::geom::Plane;
use crate::json::{self, obj, Json};
use crate::kernel::{EdgeRef, FaceRef};
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
        (
            "view",
            obj([
                ("hidden_bodies", names_to_json(&doc.view.hidden_bodies)),
                (
                    "active_component",
                    doc.view.active_component.as_deref().map(Json::from).unwrap_or(Json::Null),
                ),
                (
                    "appearances",
                    Json::Arr(
                        doc.view
                            .appearances
                            .iter()
                            .map(|(b, m)| obj([("body", b.as_str().into()), ("material", m.as_str().into())]))
                            .collect(),
                    ),
                ),
                ("hidden_features", doc.view.hidden_features.iter().map(|f| f.0 as usize).collect::<Vec<_>>().into()),
            ]),
        ),
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

fn face_to_json(f: &FaceRef) -> Json {
    obj([("point", f.point.into()), ("normal", f.normal.into())])
}

fn plane_ref_to_json(p: &PlaneRef) -> Json {
    match p {
        PlaneRef::Origin(o) => obj([("origin", o.label().into())]),
        PlaneRef::Face(f) => obj([("face", face_to_json(f))]),
        PlaneRef::Construction(id) => obj([("construction", id.0.into())]),
    }
}

fn axis_ref_to_json(a: &AxisRef) -> Json {
    match a {
        AxisRef::World(i) => obj([("world", (*i).into())]),
        AxisRef::Construction(id) => obj([("construction", id.0.into())]),
    }
}

fn profile_to_json(p: &ProfileRef) -> Json {
    obj([("sketch", p.sketch.0.into()), ("point", p.point.into())])
}

fn names_to_json(names: &[String]) -> Json {
    Json::Arr(names.iter().map(|n| n.as_str().into()).collect())
}

fn combine_op_name(op: CombineOp) -> &'static str {
    match op {
        CombineOp::Join => "join",
        CombineOp::Cut => "cut",
        CombineOp::Intersect => "intersect",
    }
}

fn feature_to_json(f: &Feature) -> Json {
    let mut fields: Vec<(String, Json)> = vec![
        ("id".into(), f.id.0.into()),
        ("name".into(), f.name.as_str().into()),
        ("suppressed".into(), f.suppressed.into()),
    ];
    if let Some(c) = &f.component {
        fields.push(("in_component".into(), c.as_str().into()));
    }
    let mut add = |k: &str, v: Json| fields.push((k.to_string(), v));
    match &f.kind {
        FeatureKind::Sketch(s) => {
            add("type", "sketch".into());
            add("plane", plane_to_json(&s.plane));
            if let Some(a) = &s.attach {
                add("attach", plane_ref_to_json(a));
            }
            if !s.projections.is_empty() {
                add(
                    "projections",
                    Json::Arr(
                        s.projections
                            .iter()
                            .map(|p| obj([("edge", p.edge.point.into()), ("entity", p.entity.0.into())]))
                            .collect(),
                    ),
                );
            }
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
                    RevolveAxis::Axis(a) => obj([("axis", axis_ref_to_json(&a))]),
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
        FeatureKind::Plane(p) => {
            add("type", "plane".into());
            add("base", plane_ref_to_json(&p.base));
            add("offset", p.offset.as_str().into());
            add("angle", p.angle.as_str().into());
        }
        FeatureKind::Axis(a) => {
            add("type", "axis".into());
            match &a.source {
                AxisSource::Edge(e) => add("edge", e.point.into()),
                AxisSource::FaceNormal(f) => add("face_normal", face_to_json(f)),
            }
        }
        FeatureKind::PressPull(p) => {
            add("type", "press_pull".into());
            add("face", face_to_json(&p.face));
            add("distance", p.distance.as_str().into());
        }
        FeatureKind::Shell(sh) => {
            add("type", "shell".into());
            add("faces", Json::Arr(sh.faces.iter().map(face_to_json).collect()));
            add("thickness", sh.thickness.as_str().into());
        }
        FeatureKind::Hole(h) => {
            add("type", "hole".into());
            add("face", face_to_json(&h.face));
            add("diameter", h.diameter.as_str().into());
            add("depth", h.depth.as_str().into());
            add("through", h.through.into());
            match &h.head {
                HoleHead::None => {}
                HoleHead::Counterbore { diameter, depth } => add(
                    "counterbore",
                    obj([("diameter", diameter.as_str().into()), ("depth", depth.as_str().into())]),
                ),
                HoleHead::Countersink { diameter, angle } => add(
                    "countersink",
                    obj([("diameter", diameter.as_str().into()), ("angle", angle.as_str().into())]),
                ),
            }
        }
        FeatureKind::Mirror(m) => {
            add("type", "mirror".into());
            add("bodies", names_to_json(&m.bodies));
            add("plane", plane_ref_to_json(&m.plane));
            add("join", m.join.into());
        }
        FeatureKind::Pattern(p) => {
            add("type", "pattern".into());
            add("bodies", names_to_json(&p.bodies));
            add("join", p.join.into());
            match &p.kind {
                PatternKind::Rectangular { direction, count, spacing, direction2, count2, spacing2 } => {
                    add("kind", "rectangular".into());
                    add("direction", axis_ref_to_json(direction));
                    add("count", count.as_str().into());
                    add("spacing", spacing.as_str().into());
                    add("direction2", axis_ref_to_json(direction2));
                    add("count2", count2.as_str().into());
                    add("spacing2", spacing2.as_str().into());
                }
                PatternKind::Circular { axis, count, angle } => {
                    add("kind", "circular".into());
                    add("axis", axis_ref_to_json(axis));
                    add("count", count.as_str().into());
                    add("angle", angle.as_str().into());
                }
            }
        }
        FeatureKind::Combine(c) => {
            add("type", "combine".into());
            add("target", c.target.as_str().into());
            add("tools", names_to_json(&c.tools));
            add("operation", combine_op_name(c.op).into());
            add("keep_tools", c.keep_tools.into());
        }
        FeatureKind::Split(sp) => {
            add("type", "split".into());
            add("body", sp.body.as_str().into());
            add("plane", plane_ref_to_json(&sp.plane));
        }
        FeatureKind::Move(m) => {
            add("type", "move".into());
            add("bodies", names_to_json(&m.bodies));
            add("translate", Json::Arr(m.translate.iter().map(|t| t.as_str().into()).collect()));
            if let Some(a) = &m.axis {
                add("axis", axis_ref_to_json(a));
            }
            add("angle", m.angle.as_str().into());
            add("copy", m.copy.into());
        }
        FeatureKind::Loft(l) => {
            add("type", "loft".into());
            add("profiles", Json::Arr(l.profiles.iter().map(profile_to_json).collect()));
            add("operation", op_name(l.operation).into());
        }
        FeatureKind::Sweep(sw) => {
            add("type", "sweep".into());
            add("profile", profile_to_json(&sw.profile));
            add("path_sketch", sw.path_sketch.0.into());
            add("path", sw.path.iter().map(|e| e.0).collect::<Vec<_>>().into());
            add("operation", op_name(sw.operation).into());
        }
        FeatureKind::Joint(j) => {
            add("type", "joint".into());
            add("moving", names_to_json(&j.moving));
            let origin = |o: &JointOrigin| match o {
                JointOrigin::Face(f) => obj([("face", face_to_json(f))]),
                JointOrigin::Edge(e) => obj([("edge", e.point.into())]),
            };
            add("from", origin(&j.from));
            add("to", origin(&j.to));
            add("kind", j.kind.label().to_lowercase().as_str().into());
            add("angle", j.angle.as_str().into());
            add("offset", j.offset.as_str().into());
            add("flip", j.flip.into());
            if let Some((lo, hi)) = &j.limits {
                add("limits", Json::Arr(vec![lo.as_str().into(), hi.as_str().into()]));
            }
        }
        FeatureKind::Component(c) => {
            add("type", "component".into());
            add("component", c.name.as_str().into());
            add("bodies", names_to_json(&c.bodies));
        }
        FeatureKind::Import(im) => {
            add("type", "import".into());
            add("path", im.path.as_str().into());
        }
        FeatureKind::Remove(r) => {
            add("type", "remove".into());
            add("bodies", names_to_json(&r.bodies));
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
        Symmetric(..) => "symmetric",
        Distance(..) => "distance",
        Length(..) => "length",
        HorizontalDistance(..) => "horizontal_distance",
        VerticalDistance(..) => "vertical_distance",
        Radius(..) => "radius",
        Diameter(..) => "diameter",
        Angle(..) => "angle",
        PointLineDistance(..) => "point_line_distance",
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
                Entity::Spline { ref points } => vec![
                    ("type".into(), "spline".into()),
                    ("points".into(), points.iter().map(|p| p.0).collect::<Vec<_>>().into()),
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

    // Optional: older files have no view state.
    let mut view = crate::document::ViewState::default();
    if let Some(v) = root.get_opt("view") {
        if let Some(b) = v.get_opt("hidden_bodies") {
            view.hidden_bodies = names_from(b)?;
        }
        if let Some(c) = v.get_opt("active_component").filter(|c| !matches!(c, Json::Null)) {
            view.active_component = Some(c.as_str()?.to_string());
        }
        if let Some(a) = v.get_opt("appearances") {
            for e in a.as_arr()? {
                view.appearances.push((str_of(e, "body")?, str_of(e, "material")?));
            }
        }
        if let Some(f) = v.get_opt("hidden_features") {
            view.hidden_features = f
                .as_arr()?
                .iter()
                .map(|x| x.as_usize().map(|n| FeatureId(n as u64)))
                .collect::<Result<_, _>>()?;
        }
    }
    Ok(Document { params, view, features, marker, next_id })
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
            let attach = f.get_opt("attach").map(plane_ref_from).transpose()?;
            let mut projections = Vec::new();
            if let Some(list) = f.get_opt("projections") {
                for p in list.as_arr()? {
                    let entity = EntityId(p.get("entity")?.as_usize()?);
                    if entity.0 >= sketch.to_data().entities.len() {
                        return Err(LoadError("projection points at a missing curve".into()));
                    }
                    projections.push(Projection { edge: EdgeRef { point: p.get("edge")?.as_vec3()? }, entity });
                }
            }
            FeatureKind::Sketch(SketchFeature { plane, attach, projections, sketch, driven })
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
                a => match a.get_opt("axis") {
                    Some(ax) => RevolveAxis::Axis(axis_ref_from(ax)?),
                    None => RevolveAxis::Line(EntityId(a.get("line")?.as_usize()?)),
                },
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
        "plane" => FeatureKind::Plane(PlaneFeature {
            base: plane_ref_from(f.get("base")?)?,
            offset: str_of(f, "offset")?,
            angle: str_of(f, "angle")?,
        }),
        "axis" => FeatureKind::Axis(AxisFeature {
            source: match (f.get_opt("edge"), f.get_opt("face_normal")) {
                (Some(e), _) => AxisSource::Edge(EdgeRef { point: e.as_vec3()? }),
                (_, Some(n)) => AxisSource::FaceNormal(face_from(n)?),
                _ => return Err(LoadError("axis needs an edge or a face".into())),
            },
        }),
        "press_pull" => FeatureKind::PressPull(PressPullFeature {
            face: face_from(f.get("face")?)?,
            distance: str_of(f, "distance")?,
        }),
        "shell" => FeatureKind::Shell(ShellFeature {
            faces: f.get("faces")?.as_arr()?.iter().map(face_from).collect::<R<_>>()?,
            thickness: str_of(f, "thickness")?,
        }),
        "hole" => FeatureKind::Hole(HoleFeature {
            face: face_from(f.get("face")?)?,
            diameter: str_of(f, "diameter")?,
            depth: str_of(f, "depth")?,
            through: f.get("through")?.as_bool()?,
            head: match (f.get_opt("counterbore"), f.get_opt("countersink")) {
                (Some(c), _) => HoleHead::Counterbore { diameter: str_of(c, "diameter")?, depth: str_of(c, "depth")? },
                (_, Some(c)) => HoleHead::Countersink { diameter: str_of(c, "diameter")?, angle: str_of(c, "angle")? },
                _ => HoleHead::None,
            },
        }),
        "mirror" => FeatureKind::Mirror(MirrorFeature {
            bodies: names_from(f.get("bodies")?)?,
            plane: plane_ref_from(f.get("plane")?)?,
            join: f.get("join")?.as_bool()?,
        }),
        "pattern" => {
            let kind = match f.get("kind")?.as_str()? {
                "rectangular" => PatternKind::Rectangular {
                    direction: axis_ref_from(f.get("direction")?)?,
                    count: str_of(f, "count")?,
                    spacing: str_of(f, "spacing")?,
                    direction2: axis_ref_from(f.get("direction2")?)?,
                    count2: str_of(f, "count2")?,
                    spacing2: str_of(f, "spacing2")?,
                },
                "circular" => PatternKind::Circular {
                    axis: axis_ref_from(f.get("axis")?)?,
                    count: str_of(f, "count")?,
                    angle: str_of(f, "angle")?,
                },
                other => return Err(LoadError(format!("unknown pattern kind '{other}'"))),
            };
            FeatureKind::Pattern(PatternFeature {
                bodies: names_from(f.get("bodies")?)?,
                kind,
                join: f.get("join")?.as_bool()?,
            })
        }
        "combine" => FeatureKind::Combine(CombineFeature {
            target: str_of(f, "target")?,
            tools: names_from(f.get("tools")?)?,
            op: match f.get("operation")?.as_str()? {
                "join" => CombineOp::Join,
                "cut" => CombineOp::Cut,
                "intersect" => CombineOp::Intersect,
                other => return Err(LoadError(format!("unknown combine operation '{other}'"))),
            },
            keep_tools: f.get("keep_tools")?.as_bool()?,
        }),
        "split" => FeatureKind::Split(SplitFeature {
            body: str_of(f, "body")?,
            plane: plane_ref_from(f.get("plane")?)?,
        }),
        "move" => {
            let t = f.get("translate")?.as_arr()?;
            let [x, y, z] = t else { return Err(LoadError("translate needs three values".into())) };
            FeatureKind::Move(MoveFeature {
                bodies: names_from(f.get("bodies")?)?,
                translate: [x.as_str()?.to_string(), y.as_str()?.to_string(), z.as_str()?.to_string()],
                axis: f.get_opt("axis").map(axis_ref_from).transpose()?,
                angle: str_of(f, "angle")?,
                copy: f.get("copy")?.as_bool()?,
            })
        }
        "loft" => FeatureKind::Loft(LoftFeature {
            profiles: f.get("profiles")?.as_arr()?.iter().map(profile_from).collect::<R<_>>()?,
            operation: op_from(f.get("operation")?)?,
        }),
        "sweep" => FeatureKind::Sweep(SweepFeature {
            profile: profile_from(f.get("profile")?)?,
            path_sketch: FeatureId(f.get("path_sketch")?.as_usize()? as u64),
            path: f
                .get("path")?
                .as_arr()?
                .iter()
                .map(|e| e.as_usize().map(EntityId))
                .collect::<Result<_, _>>()?,
            operation: op_from(f.get("operation")?)?,
        }),
        "remove" => FeatureKind::Remove(RemoveFeature { bodies: names_from(f.get("bodies")?)? }),
        "import" => FeatureKind::Import(ImportFeature { path: str_of(f, "path")? }),
        "component" => FeatureKind::Component(ComponentFeature {
            name: str_of(f, "component")?,
            bodies: names_from(f.get("bodies")?)?,
        }),
        "joint" => {
            let origin = |j: &Json| -> R<JointOrigin> {
                match (j.get_opt("face"), j.get_opt("edge")) {
                    (Some(fc), _) => Ok(JointOrigin::Face(face_from(fc)?)),
                    (_, Some(e)) => Ok(JointOrigin::Edge(EdgeRef { point: e.as_vec3()? })),
                    _ => Err(LoadError("joint origin needs a face or an edge".into())),
                }
            };
            let kind_name = f.get("kind")?.as_str()?;
            let kind = JointKind::ALL
                .into_iter()
                .find(|k| k.label().to_lowercase() == kind_name)
                .ok_or_else(|| LoadError(format!("unknown joint kind '{kind_name}'")))?;
            FeatureKind::Joint(JointFeature {
                moving: names_from(f.get("moving")?)?,
                from: origin(f.get("from")?)?,
                to: origin(f.get("to")?)?,
                kind,
                angle: str_of(f, "angle")?,
                offset: str_of(f, "offset")?,
                flip: f.get("flip")?.as_bool()?,
                limits: match f.get_opt("limits") {
                    None => None,
                    Some(l) => match l.as_arr()? {
                        [lo, hi] => Some((lo.as_str()?.to_string(), hi.as_str()?.to_string())),
                        _ => return Err(LoadError("joint limits need [min, max]".into())),
                    },
                },
            })
        }
        other => return Err(LoadError(format!("unknown feature type '{other}'"))),
    };
    let component = f.get_opt("in_component").map(|c| c.as_str().map(str::to_string)).transpose()?;
    Ok(Feature { id, name, suppressed, kind, component })
}

fn str_of(f: &Json, key: &str) -> R<String> {
    Ok(f.get(key)?.as_str()?.to_string())
}

fn names_from(j: &Json) -> R<Vec<String>> {
    Ok(j.as_arr()?.iter().map(|n| n.as_str().map(str::to_string)).collect::<Result<_, _>>()?)
}

fn face_from(j: &Json) -> R<FaceRef> {
    Ok(FaceRef { point: j.get("point")?.as_vec3()?, normal: j.get("normal")?.as_vec3()? })
}

fn plane_ref_from(j: &Json) -> R<PlaneRef> {
    if let Some(o) = j.get_opt("origin") {
        let name = o.as_str()?;
        return OriginPlane::ALL
            .into_iter()
            .find(|p| p.label() == name)
            .map(PlaneRef::Origin)
            .ok_or_else(|| LoadError(format!("unknown origin plane '{name}'")));
    }
    if let Some(f) = j.get_opt("face") {
        return Ok(PlaneRef::Face(face_from(f)?));
    }
    Ok(PlaneRef::Construction(FeatureId(j.get("construction")?.as_usize()? as u64)))
}

fn axis_ref_from(j: &Json) -> R<AxisRef> {
    if let Some(w) = j.get_opt("world") {
        let i = w.as_usize()?;
        if i > 2 {
            return Err(LoadError("world axis must be 0, 1 or 2".into()));
        }
        return Ok(AxisRef::World(i));
    }
    Ok(AxisRef::Construction(FeatureId(j.get("construction")?.as_usize()? as u64)))
}

fn profile_from(j: &Json) -> R<ProfileRef> {
    Ok(ProfileRef { sketch: FeatureId(j.get("sketch")?.as_usize()? as u64), point: j.get("point")?.as_vec2()? })
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
                "spline" => Entity::Spline {
                    points: e
                        .get("points")?
                        .as_arr()?
                        .iter()
                        .map(|p| p.as_usize().map(PointId))
                        .collect::<Result<_, _>>()?,
                },
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
        ("symmetric", (2, 1, false)) => Symmetric(p[0], p[1], e[0]),
        ("distance", (2, 0, true)) => Distance(p[0], p[1], v.unwrap()),
        ("length", (0, 1, true)) => Length(e[0], v.unwrap()),
        ("horizontal_distance", (2, 0, true)) => HorizontalDistance(p[0], p[1], v.unwrap()),
        ("vertical_distance", (2, 0, true)) => VerticalDistance(p[0], p[1], v.unwrap()),
        ("radius", (0, 1, true)) => Radius(e[0], v.unwrap()),
        ("diameter", (0, 1, true)) => Diameter(e[0], v.unwrap()),
        ("angle", (0, 2, true)) => Angle(e[0], e[1], v.unwrap()),
        ("point_line_distance", (1, 1, true)) => PointLineDistance(p[0], e[0], v.unwrap()),
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
            Concentric(c, c), Symmetric(a, b, l2), Distance(a, b, 1.5), Length(l1, 0.1 + 0.2), HorizontalDistance(a, b, -2.0),
            VerticalDistance(a, b, 3.0), Radius(c, 4.0), Diameter(c, 8.0), Angle(l1, l2, 0.7), PointLineDistance(a, l2, 2.5),
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
