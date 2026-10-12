// ferrocad additions: complete modelling operations that never let an OCCT
// exception escape. Every failure (Standard_Failure, IsDone() == false, an
// invalid result) becomes a std::runtime_error, which cxx turns into a Rust
// `Err` for functions declared `-> Result<…>`. Without this, OCCT throwing
// through a cxx function marked noexcept aborts the whole process.
#pragma once
#include "rust/cxx.h"

#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepOffsetAPI_MakePipe.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <GProp_GProps.hxx>
#include <IntCurvesFace_ShapeIntersector.hxx>
#include <LocOpe_DPrism.hxx>
#include <STEPControl_Reader.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Lin.hxx>
#include <gp_Pln.hxx>
#include <gp_Trsf.hxx>

namespace ferro {

[[noreturn]] inline void fail(const std::string &what) { throw std::runtime_error(what); }

/// Run `f`, check its result and translate OCCT exceptions.
template <typename F> std::unique_ptr<TopoDS_Shape> guard(const char *op, F &&f) {
  try {
    TopoDS_Shape s = f();
    if (s.IsNull()) {
      fail(std::string(op) + " produced no shape");
    }
    BRepCheck_Analyzer check(s);
    if (!check.IsValid()) {
      fail(std::string(op) + " produced an invalid shape");
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(s));
  } catch (const Standard_Failure &e) {
    const char *msg = e.GetMessageString();
    fail(std::string(op) + " failed" + ((msg && *msg) ? std::string(": ") + msg : std::string()));
  }
}

inline std::vector<TopoDS_Edge> edges_of(const TopoDS_Shape &shape) {
  std::vector<TopoDS_Edge> out;
  for (TopExp_Explorer ex(shape, TopAbs_EDGE); ex.More(); ex.Next()) {
    out.push_back(TopoDS::Edge(ex.Current()));
  }
  return out;
}

inline std::vector<TopoDS_Face> faces_of(const TopoDS_Shape &shape) {
  std::vector<TopoDS_Face> out;
  for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
    out.push_back(TopoDS::Face(ex.Current()));
  }
  return out;
}

/// Reads the flat curve encoding produced by the Rust side.
struct Reader {
  rust::Slice<const double> data;
  size_t i;
  explicit Reader(rust::Slice<const double> d) : data(d), i(0) {}
  double next() {
    if (i >= data.size()) {
      fail("malformed geometry data");
    }
    return data[i++];
  }
  gp_Pnt pnt() {
    double x = next(), y = next(), z = next();
    return gp_Pnt(x, y, z);
  }
  gp_Dir dir() {
    double x = next(), y = next(), z = next();
    return gp_Dir(x, y, z);
  }
  /// nseg, then per segment: 0 line (a, b) · 1 arc (start, mid, end) ·
  /// 2 circle (center, normal, radius) · 3 spline (n, points…)
  TopoDS_Wire wire() {
    BRepBuilderAPI_MakeWire mw;
    int n = static_cast<int>(next());
    for (int s = 0; s < n; ++s) {
      int type = static_cast<int>(next());
      TopoDS_Edge e;
      if (type == 0) {
        gp_Pnt a = pnt(), b = pnt();
        if (a.Distance(b) < 1e-9) {
          fail("zero-length line in profile");
        }
        e = BRepBuilderAPI_MakeEdge(a, b);
      } else if (type == 1) {
        gp_Pnt a = pnt(), m = pnt(), b = pnt();
        GC_MakeArcOfCircle arc(a, m, b);
        if (!arc.IsDone()) {
          fail("invalid arc in profile");
        }
        e = BRepBuilderAPI_MakeEdge(arc.Value());
      } else if (type == 3) {
        int np = static_cast<int>(next());
        if (np < 2) {
          fail("spline needs two points");
        }
        Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, np);
        for (int k = 1; k <= np; ++k) {
          pts->SetValue(k, pnt());
        }
        GeomAPI_Interpolate interp(pts, false, 1e-7);
        interp.Perform();
        if (!interp.IsDone()) {
          fail("could not build the spline");
        }
        e = BRepBuilderAPI_MakeEdge(interp.Curve());
      } else {
        gp_Pnt c = pnt();
        gp_Dir d = dir();
        double r = next();
        if (r <= 0) {
          fail("circle with zero radius");
        }
        e = BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(c, d), r));
      }
      mw.Add(e);
      if (!mw.IsDone()) {
        fail("profile curves are not connected");
      }
    }
    if (!mw.IsDone()) {
      fail("empty profile");
    }
    return mw.Wire();
  }
  /// nloops, then loops: the first is the outer boundary, the rest holes.
  TopoDS_Face face() {
    int loops = static_cast<int>(next());
    if (loops < 1) {
      fail("empty profile");
    }
    BRepBuilderAPI_MakeFace mf(wire(), true);
    if (!mf.IsDone()) {
      fail("profile is not planar");
    }
    for (int l = 1; l < loops; ++l) {
      mf.Add(wire());
    }
    if (!mf.IsDone()) {
      fail("could not build the profile face");
    }
    ShapeFix_Face fix(mf.Face());
    fix.FixOrientation();
    fix.Perform();
    return fix.Face();
  }
};

} // namespace ferro

inline std::unique_ptr<TopoDS_Shape> ferro_fillet(const TopoDS_Shape &shape, rust::Slice<const uint32_t> edges,
                                                  double radius, bool chamfer) {
  return ferro::guard(chamfer ? "chamfer" : "fillet", [&]() -> TopoDS_Shape {
    auto all = ferro::edges_of(shape);
    if (chamfer) {
      BRepFilletAPI_MakeChamfer mk(shape);
      for (auto i : edges) {
        if (i >= all.size()) {
          ferro::fail("edge index out of range");
        }
        mk.Add(radius, all[i]);
      }
      mk.Build();
      if (!mk.IsDone()) {
        ferro::fail("chamfer failed – the distance may be too large for the adjacent faces");
      }
      return mk.Shape();
    }
    BRepFilletAPI_MakeFillet mk(shape);
    for (auto i : edges) {
      if (i >= all.size()) {
        ferro::fail("edge index out of range");
      }
      mk.Add(radius, all[i]);
    }
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("fillet failed – the radius may be too large for the adjacent faces");
    }
    return mk.Shape();
  });
}

/// Chamfer with two distances: `d1` on the first face adjacent to each edge,
/// `d2` on the other one.
inline std::unique_ptr<TopoDS_Shape> ferro_chamfer2(const TopoDS_Shape &shape, rust::Slice<const uint32_t> edges,
                                                    double d1, double d2) {
  return ferro::guard("chamfer", [&]() -> TopoDS_Shape {
    auto all = ferro::edges_of(shape);
    TopTools_IndexedDataMapOfShapeListOfShape faces;
    TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, faces);
    BRepFilletAPI_MakeChamfer mk(shape);
    for (auto i : edges) {
      if (i >= all.size()) {
        ferro::fail("edge index out of range");
      }
      const TopTools_ListOfShape *adjacent = faces.Seek(all[i]);
      if (adjacent == nullptr || adjacent->IsEmpty()) {
        ferro::fail("edge has no adjacent face");
      }
      mk.Add(d1, d2, all[i], TopoDS::Face(adjacent->First()));
    }
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("chamfer failed – the distances may be too large for the adjacent faces");
    }
    return mk.Shape();
  });
}

/// Draft the given faces by p[6] radians around the neutral plane through
/// p[0..3] whose normal p[3..6] is the pull direction.
inline std::unique_ptr<TopoDS_Shape> ferro_draft(const TopoDS_Shape &shape, rust::Slice<const uint32_t> faces,
                                                 rust::Slice<const double> p) {
  return ferro::guard("draft", [&]() -> TopoDS_Shape {
    if (p.size() < 7) {
      ferro::fail("draft needs 7 values");
    }
    auto all = ferro::faces_of(shape);
    gp_Dir dir(p[3], p[4], p[5]);
    gp_Pln neutral(gp_Pnt(p[0], p[1], p[2]), dir);
    BRepOffsetAPI_DraftAngle mk(shape);
    for (auto i : faces) {
      if (i >= all.size()) {
        ferro::fail("face index out of range");
      }
      mk.Add(all[i], dir, p[6], neutral);
      if (!mk.AddDone()) {
        ferro::fail("this face cannot be drafted in that direction");
      }
    }
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("draft failed – the angle may be too large");
    }
    return mk.Shape();
  });
}

/// op: 0 union, 1 subtract, 2 intersect. Coplanar faces are merged afterwards.
inline std::unique_ptr<TopoDS_Shape> ferro_boolean(const TopoDS_Shape &a, const TopoDS_Shape &b, uint32_t op) {
  return ferro::guard("boolean", [&]() -> TopoDS_Shape {
    TopoDS_Shape r;
    bool ok = false;
    if (op == 0) {
      BRepAlgoAPI_Fuse mk(a, b);
      ok = mk.IsDone() && !mk.HasErrors();
      if (ok) r = mk.Shape();
    } else if (op == 1) {
      BRepAlgoAPI_Cut mk(a, b);
      ok = mk.IsDone() && !mk.HasErrors();
      if (ok) r = mk.Shape();
    } else {
      BRepAlgoAPI_Common mk(a, b);
      ok = mk.IsDone() && !mk.HasErrors();
      if (ok) r = mk.Shape();
    }
    if (!ok) {
      ferro::fail("boolean operation failed");
    }
    ShapeUpgrade_UnifySameDomain unify(r, true, true, false);
    unify.Build();
    return unify.Shape();
  });
}

/// Hollow the solid, removing the given faces; walls go inwards.
inline std::unique_ptr<TopoDS_Shape> ferro_shell(const TopoDS_Shape &shape, rust::Slice<const uint32_t> faces,
                                                 double thickness) {
  return ferro::guard("shell", [&]() -> TopoDS_Shape {
    auto all = ferro::faces_of(shape);
    TopTools_ListOfShape list;
    for (auto i : faces) {
      if (i >= all.size()) {
        ferro::fail("face index out of range");
      }
      list.Append(all[i]);
    }
    BRepOffsetAPI_MakeThickSolid mk;
    mk.MakeThickSolidByJoin(shape, list, -thickness, 1e-3);
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("shell failed – the thickness may be too large");
    }
    return mk.Shape();
  });
}

/// kind 0: translate by p[3..6] · 1: rotate around axis (p[0..3], p[3..6]) by
/// p[6] radians · 2: mirror at the plane through p[0..3] with normal p[3..6].
inline std::unique_ptr<TopoDS_Shape> ferro_transform(const TopoDS_Shape &shape, uint32_t kind,
                                                     rust::Slice<const double> p) {
  return ferro::guard("transform", [&]() -> TopoDS_Shape {
    if (p.size() < 7) {
      ferro::fail("transform needs 7 values");
    }
    gp_Trsf t;
    if (kind == 0) {
      t.SetTranslation(gp_Vec(p[3], p[4], p[5]));
    } else if (kind == 1) {
      t.SetRotation(gp_Ax1(gp_Pnt(p[0], p[1], p[2]), gp_Dir(p[3], p[4], p[5])), p[6]);
    } else {
      t.SetMirror(gp_Ax2(gp_Pnt(p[0], p[1], p[2]), gp_Dir(p[3], p[4], p[5])));
    }
    BRepBuilderAPI_Transform mk(shape, t, true);
    return mk.Shape();
  });
}

/// A planar face from encoded loops (see ferro::Reader::face).
inline std::unique_ptr<TopoDS_Shape> ferro_profile_face(rust::Slice<const double> data) {
  return ferro::guard("profile", [&]() -> TopoDS_Shape {
    ferro::Reader r(data);
    return r.face();
  });
}

inline std::unique_ptr<TopoDS_Shape> ferro_prism(const TopoDS_Shape &face, double dx, double dy, double dz) {
  return ferro::guard("extrude", [&]() -> TopoDS_Shape {
    BRepPrimAPI_MakePrism mk(face, gp_Vec(dx, dy, dz));
    if (!mk.IsDone()) {
      ferro::fail("extrude failed");
    }
    return mk.Shape();
  });
}

/// Tapered prism of a planar face: `h1` along the face normal, `h2` the
/// other way, side faces drafted by `angle` (radians). `reverse` flips the face first.
inline std::unique_ptr<TopoDS_Shape> ferro_draft_prism(const TopoDS_Shape &face, double h1, double h2, double angle,
                                                       bool reverse) {
  return ferro::guard("tapered extrude", [&]() -> TopoDS_Shape {
    TopoDS_Face f = TopoDS::Face(face);
    if (reverse) {
      f = TopoDS::Face(f.Reversed());
    }
    LocOpe_DPrism mk(f, h1, h2, angle);
    if (!mk.IsDone()) {
      ferro::fail("tapered extrude failed – the taper may be too steep");
    }
    return mk.Shape();
  });
}

/// p: axis origin (3), axis direction (3), angle in radians.
inline std::unique_ptr<TopoDS_Shape> ferro_revol(const TopoDS_Shape &face, rust::Slice<const double> p) {
  return ferro::guard("revolve", [&]() -> TopoDS_Shape {
    if (p.size() < 7) {
      ferro::fail("revolve needs 7 values");
    }
    gp_Ax1 axis(gp_Pnt(p[0], p[1], p[2]), gp_Dir(p[3], p[4], p[5]));
    BRepPrimAPI_MakeRevol mk(face, axis, p[6]);
    if (!mk.IsDone()) {
      ferro::fail("revolve failed – does the profile cross the axis?");
    }
    return mk.Shape();
  });
}

/// data: nwires, then wires (see ferro::Reader::wire), in loft order.
inline std::unique_ptr<TopoDS_Shape> ferro_loft(rust::Slice<const double> data) {
  return ferro::guard("loft", [&]() -> TopoDS_Shape {
    ferro::Reader r(data);
    int n = static_cast<int>(r.next());
    if (n < 2) {
      ferro::fail("loft needs at least two profiles");
    }
    BRepOffsetAPI_ThruSections mk(true, false);
    for (int i = 0; i < n; ++i) {
      mk.AddWire(r.wire());
    }
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("loft failed");
    }
    return mk.Shape();
  });
}

/// Sweep a profile face along an open path wire (see ferro::Reader::wire).
inline std::unique_ptr<TopoDS_Shape> ferro_sweep(const TopoDS_Shape &face, rust::Slice<const double> path) {
  return ferro::guard("sweep", [&]() -> TopoDS_Shape {
    ferro::Reader r(path);
    TopoDS_Wire spine = r.wire();
    BRepOffsetAPI_MakePipe mk(spine, face);
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("sweep failed");
    }
    return mk.Shape();
  });
}

/// kind 0: face, 1: edge, 2: solid – by index in explorer order.
inline std::unique_ptr<TopoDS_Shape> ferro_sub_shape(const TopoDS_Shape &shape, uint32_t kind, uint32_t index) {
  return ferro::guard("select", [&]() -> TopoDS_Shape {
    uint32_t i = 0;
    TopAbs_ShapeEnum k = kind == 0 ? TopAbs_FACE : kind == 1 ? TopAbs_EDGE : TopAbs_SOLID;
    for (TopExp_Explorer ex(shape, k); ex.More(); ex.Next(), ++i) {
      if (i == index) {
        return ex.Current();
      }
    }
    ferro::fail("sub-shape index out of range");
  });
}

/// Find the face hit by the line through q[0..3] along q[3..6] whose outward
/// normal points along q[3..6], nearest to q[0..3]. Returns
/// [index, px, py, pz, nx, ny, nz, planar] or an empty vector.
inline rust::Vec<double> ferro_find_face(const TopoDS_Shape &shape, rust::Slice<const double> q) {
  rust::Vec<double> out;
  try {
    if (q.size() < 6) {
      ferro::fail("find_face needs 6 values");
    }
    gp_Pnt p(q[0], q[1], q[2]);
    gp_Dir d(q[3], q[4], q[5]);
    IntCurvesFace_ShapeIntersector inter;
    inter.Load(shape, 1e-7);
    inter.Perform(gp_Lin(p, d), -1e12, 1e12);
    if (!inter.IsDone()) {
      return out;
    }
    auto faces = ferro::faces_of(shape);
    double best = 1e300;
    for (int i = 1; i <= inter.NbPnt(); ++i) {
      const TopoDS_Face &f = inter.Face(i);
      BRepAdaptor_Surface surf(f);
      gp_Pnt hit;
      gp_Vec du, dv;
      surf.D1(inter.UParameter(i), inter.VParameter(i), hit, du, dv);
      gp_Vec n = du.Crossed(dv);
      if (n.Magnitude() < 1e-12) {
        continue;
      }
      n.Normalize();
      if (f.Orientation() == TopAbs_REVERSED) {
        n.Reverse();
      }
      if (n.Dot(gp_Vec(d)) < 0.9) {
        continue;
      }
      double dist = std::abs(inter.WParameter(i));
      if (dist >= best) {
        continue;
      }
      for (size_t k = 0; k < faces.size(); ++k) {
        if (faces[k].IsSame(f)) {
          best = dist;
          out.clear();
          for (double v : {static_cast<double>(k), hit.X(), hit.Y(), hit.Z(), n.X(), n.Y(), n.Z(),
                           surf.GetType() == GeomAbs_Plane ? 1.0 : 0.0}) {
            out.push_back(v);
          }
          break;
        }
      }
    }
  } catch (const Standard_Failure &) {
    out.clear();
  }
  return out;
}

/// Read a STEP file. Imported data is often slightly imperfect, so the result
/// is not run through the validity check.
inline std::unique_ptr<TopoDS_Shape> ferro_read_step(rust::Str path) {
  try {
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone) {
      ferro::fail("could not read the STEP file");
    }
    reader.TransferRoots();
    TopoDS_Shape s = reader.OneShape();
    if (s.IsNull()) {
      ferro::fail("the STEP file contains no shape");
    }
    return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(s));
  } catch (const Standard_Failure &e) {
    const char *msg = e.GetMessageString();
    ferro::fail(std::string("STEP import failed") + ((msg && *msg) ? std::string(": ") + msg : std::string()));
  }
}

/// [volume, area, centre x, y, z]
inline rust::Vec<double> ferro_properties(const TopoDS_Shape &shape) {
  rust::Vec<double> out;
  try {
    GProp_GProps vol, area;
    BRepGProp::VolumeProperties(shape, vol);
    BRepGProp::SurfaceProperties(shape, area);
    gp_Pnt c = vol.CentreOfMass();
    for (double v : {vol.Mass(), area.Mass(), c.X(), c.Y(), c.Z()}) {
      out.push_back(v);
    }
  } catch (const Standard_Failure &) {
    out.clear();
  }
  return out;
}

/// Number of sub-shapes: kind 0 faces, 1 edges, 2 solids.
inline uint32_t ferro_count(const TopoDS_Shape &shape, uint32_t kind) {
  TopAbs_ShapeEnum k = kind == 0 ? TopAbs_FACE : kind == 1 ? TopAbs_EDGE : TopAbs_SOLID;
  uint32_t n = 0;
  for (TopExp_Explorer ex(shape, k); ex.More(); ex.Next()) {
    ++n;
  }
  return n;
}
