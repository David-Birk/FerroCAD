#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Builder.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Wire.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Vertex.hxx>
#include <bindings_common.hxx>

// OCCT 7.9 turned `TopoDS` from a class into a namespace, which cxx cannot bind as
// a `Self` type. This struct forwards to it and works with both 7.8 and 7.9.
struct TopoDS_Cast {
  static const TopoDS_Vertex &Vertex(const TopoDS_Shape &s) { return TopoDS::Vertex(s); }
  static const TopoDS_Edge &Edge(const TopoDS_Shape &s) { return TopoDS::Edge(s); }
  static const TopoDS_Wire &Wire(const TopoDS_Shape &s) { return TopoDS::Wire(s); }
  static const TopoDS_Face &Face(const TopoDS_Shape &s) { return TopoDS::Face(s); }
  static const TopoDS_Shell &Shell(const TopoDS_Shape &s) { return TopoDS::Shell(s); }
  static const TopoDS_Solid &Solid(const TopoDS_Shape &s) { return TopoDS::Solid(s); }
  static const TopoDS_Compound &Compound(const TopoDS_Shape &s) { return TopoDS::Compound(s); }
};
