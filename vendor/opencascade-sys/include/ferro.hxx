// ferrocad additions: complete modelling operations that never let an OCCT
// exception escape. Every failure (Standard_Failure, IsDone() == false, an
// invalid result) becomes a std::runtime_error, which cxx turns into a Rust
// `Err` for functions declared `-> Result<…>`. Without this, OCCT throwing
// through a cxx function marked noexcept aborts the whole process.
#pragma once
#include "rust/cxx.h"

#include <algorithm>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBndLib.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <Bnd_Box.hxx>
#include <GCE2d_MakeSegment.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <gp_Lin2d.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
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
#include <ShapeFix_Shape.hxx>
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

/// Outward normal of a planar face.
inline gp_Dir plane_normal(const TopoDS_Face &x) {
  gp_Dir n = BRepAdaptor_Surface(x).Plane().Axis().Direction();
  return x.Orientation() == TopAbs_REVERSED ? n.Reversed() : n;
}

/// `x` turned to face the same way as `like`, so prisms don't turn inside
/// out and offsets grow outwards.
inline TopoDS_Face aligned(TopoDS_Face x, const TopoDS_Face &like) {
  if (plane_normal(x).Dot(plane_normal(like)) < 0.0) {
    x.Reverse();
  }
  return x;
}

/// The region of planar `face` grown (d > 0) or shrunk (d < 0) by |d|, with
/// round corners outside. Null if it shrinks away.
inline TopoDS_Shape offset_region(const TopoDS_Face &face, double d) {
  BRepOffsetAPI_MakeOffset mk(face, GeomAbs_Arc);
  mk.Perform(d);
  if (!mk.IsDone()) {
    return TopoDS_Shape();
  }
  // The largest wire bounds the region, the others are its holes.
  std::vector<std::pair<double, TopoDS_Wire>> wires;
  for (TopExp_Explorer ex(mk.Shape(), TopAbs_WIRE); ex.More(); ex.Next()) {
    TopoDS_Wire w = TopoDS::Wire(ex.Current());
    BRepBuilderAPI_MakeFace mf(w, true);
    if (!mf.IsDone()) {
      continue;
    }
    GProp_GProps g;
    BRepGProp::SurfaceProperties(mf.Face(), g);
    wires.push_back(std::make_pair(std::abs(g.Mass()), w));
  }
  if (wires.empty() || wires[0].first < 1e-12) {
    return TopoDS_Shape();
  }
  std::sort(wires.begin(), wires.end(),
            [](const std::pair<double, TopoDS_Wire> &a, const std::pair<double, TopoDS_Wire> &b) { return a.first > b.first; });
  BRepBuilderAPI_MakeFace mf(wires[0].second, true);
  for (size_t i = 1; i < wires.size(); ++i) {
    mf.Add(wires[i].second);
  }
  if (!mf.IsDone()) {
    return TopoDS_Shape();
  }
  // Holes may come out with the wrong orientation; ShapeFix repairs that,
  // but it can spoil a valid face, so only use it when needed.
  TopoDS_Face region = mf.Face();
  if (!BRepCheck_Analyzer(region).IsValid()) {
    ShapeFix_Face fix(region);
    fix.FixOrientation();
    fix.Perform();
    region = fix.Face();
  }
  return aligned(region, face);
}

} // namespace ferro

/// One band of a thin extrude: the planar single-loop `face` grown by
/// p[5] minus it shrunk by p[6], extruded from p[3] against the unit
/// direction p[0..3] to p[4] along it.
inline std::unique_ptr<TopoDS_Shape> ferro_thin_band(const TopoDS_Shape &face, rust::Slice<const double> p) {
  return ferro::guard("thin extrude", [&]() -> TopoDS_Shape {
    if (p.size() < 7) {
      ferro::fail("thin extrude needs 7 values");
    }
    TopoDS_Face f = TopoDS::Face(face);
    gp_Vec dir(p[0], p[1], p[2]);
    double back = p[3], fwd = p[4], grow = p[5], shrink = p[6];
    auto prism = [&](const TopoDS_Shape &region) -> TopoDS_Shape {
      gp_Trsf move;
      move.SetTranslation(-dir * back);
      BRepBuilderAPI_Transform moved(region, move, true);
      BRepPrimAPI_MakePrism mk(moved.Shape(), dir * (fwd + back));
      if (!mk.IsDone()) {
        ferro::fail("thin extrude failed");
      }
      return mk.Shape();
    };
    TopoDS_Shape big = grow > 0.0 ? ferro::offset_region(f, grow) : TopoDS_Shape(f);
    if (big.IsNull()) {
      ferro::fail("thin extrude failed – could not offset the profile");
    }
    TopoDS_Shape small = shrink > 0.0 ? ferro::offset_region(f, -shrink) : TopoDS_Shape(f);
    if (small.IsNull()) {
      return prism(big); // the wall fills the loop
    }
    BRepAlgoAPI_Cut cut(prism(big), prism(small));
    if (!cut.IsDone() || cut.HasErrors()) {
      ferro::fail("thin extrude failed");
    }
    return cut.Shape();
  });
}

/// Cylinder data of face `index`: [origin(3), axis(3), radius, v_min, v_max,
/// outward] where v is the position along the axis from origin and outward
/// is 1 when the face normal points away from the axis (an outer surface).
/// Empty if the face is not a cylinder.
inline rust::Vec<double> ferro_cylinder(const TopoDS_Shape &shape, uint32_t index) {
  rust::Vec<double> out;
  try {
    auto faces = ferro::faces_of(shape);
    if (index >= faces.size()) {
      return out;
    }
    const TopoDS_Face &f = faces[index];
    BRepAdaptor_Surface s(f);
    if (s.GetType() != GeomAbs_Cylinder) {
      return out;
    }
    gp_Cylinder c = s.Cylinder();
    gp_Ax1 axis = c.Axis();
    double u = (s.FirstUParameter() + s.LastUParameter()) / 2.0;
    double v = (s.FirstVParameter() + s.LastVParameter()) / 2.0;
    gp_Pnt p;
    gp_Vec du, dv;
    s.D1(u, v, p, du, dv);
    gp_Vec n = du.Crossed(dv);
    if (f.Orientation() == TopAbs_REVERSED) {
      n.Reverse();
    }
    gp_Vec radial(axis.Location(), p);
    gp_Vec along(axis.Direction());
    radial -= along * radial.Dot(along);
    bool outward = n.Dot(radial) > 0.0;
    // Axial extent from the face's bounding box (V bounds can be infinite).
    Bnd_Box box;
    BRepBndLib::AddOptimal(f, box, false, false);
    double x0, y0, z0, x1, y1, z1;
    box.Get(x0, y0, z0, x1, y1, z1);
    double lo = 1e300, hi = -1e300;
    for (int k = 0; k < 8; ++k) {
      gp_Pnt q(k & 1 ? x1 : x0, k & 2 ? y1 : y0, k & 4 ? z1 : z0);
      double t = gp_Vec(axis.Location(), q).Dot(along);
      lo = std::min(lo, t);
      hi = std::max(hi, t);
    }
    gp_Pnt o = axis.Location();
    gp_Dir d = axis.Direction();
    for (double x : {o.X(), o.Y(), o.Z(), d.X(), d.Y(), d.Z(), c.Radius(), lo, hi, outward ? 1.0 : 0.0}) {
      out.push_back(x);
    }
  } catch (const Standard_Failure &) {
    out.clear();
  }
  return out;
}

/// Axis-aligned bounds of a shape: [xmin, ymin, zmin, xmax, ymax, zmax].
inline rust::Vec<double> ferro_bounds(const TopoDS_Shape &shape) {
  rust::Vec<double> out;
  try {
    Bnd_Box box;
    BRepBndLib::AddOptimal(shape, box, false, false);
    if (box.IsVoid()) {
      return out;
    }
    double x0, y0, z0, x1, y1, z1;
    box.Get(x0, y0, z0, x1, y1, z1);
    for (double x : {x0, y0, z0, x1, y1, z1}) {
      out.push_back(x);
    }
  } catch (const Standard_Failure &) {
    out.clear();
  }
  return out;
}

/// Helical V groove for a thread: p = origin(3), axis(3), base radius,
/// apex radius, pitch, from, to (axial positions). The 60° triangle has its
/// base on the base radius (just outside the material) and its apex at the
/// apex radius; it is swept along a helix on the base radius.
inline std::unique_ptr<TopoDS_Shape> ferro_thread_groove(rust::Slice<const double> p) {
  return ferro::guard("thread", [&]() -> TopoDS_Shape {
    if (p.size() < 11) {
      ferro::fail("thread needs 11 values");
    }
    gp_Pnt o(p[0], p[1], p[2]);
    gp_Dir d(p[3], p[4], p[5]);
    double rb = p[6], ra = p[7], pitch = p[8], from = p[9], to = p[10];
    if (pitch <= 0.0 || to <= from || rb <= 0.0 || ra <= 0.0) {
      ferro::fail("invalid thread size");
    }
    gp_Ax3 ax(o, d);
    gp_Dir x = ax.XDirection(), y = ax.YDirection();
    // The helix on the base radius, interpolated through 24 points per turn.
    double turns = (to - from) / pitch;
    int n = std::max(8, static_cast<int>(std::ceil(turns * 24.0)));
    Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, n + 1);
    for (int i = 0; i <= n; ++i) {
      double t = turns * i / n;
      double a = 2.0 * M_PI * t;
      gp_XYZ q = o.XYZ() + x.XYZ() * (rb * std::cos(a)) + y.XYZ() * (rb * std::sin(a)) + d.XYZ() * (from + pitch * t);
      pts->SetValue(i + 1, gp_Pnt(q));
    }
    GeomAPI_Interpolate interp(pts, false, 1e-7);
    interp.Perform();
    if (!interp.IsDone()) {
      ferro::fail("could not build the thread helix");
    }
    TopoDS_Wire spine = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(interp.Curve()));
    // 60° flanks: half width = depth · tan 30°.
    double depth = std::abs(ra - rb);
    double half = depth * std::tan(M_PI / 6.0);
    auto at = [&](double r, double h) {
      return gp_Pnt(o.XYZ() + gp_Vec(x).XYZ() * r + gp_Vec(d).XYZ() * h);
    };
    BRepBuilderAPI_MakePolygon tri(at(rb, from - half), at(rb, from + half), at(ra, from), true);
    BRepOffsetAPI_MakePipeShell mk(spine);
    mk.SetMode(d); // keep the profile in the axial plane
    mk.Add(tri.Wire());
    mk.Build();
    if (!mk.IsDone()) {
      ferro::fail("thread sweep failed");
    }
    if (!mk.MakeSolid()) {
      ferro::fail("thread sweep is not closed");
    }
    return mk.Shape();
  });
}

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
