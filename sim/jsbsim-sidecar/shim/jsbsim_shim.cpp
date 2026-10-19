// The C surface of JSBSim that the Rust sidecar calls.
//
// Every function catches every C++ exception, so no exception crosses
// into Rust. An engine is one FGFDMExec; properties are addressed by an
// index into a table of property nodes, so a step reads and writes
// numbers, not strings. The ground is a callback into Rust: JSBSim asks
// for the terrain height under a geodetic point, and the shim turns the
// answer into the contact point and surface normal JSBSim needs.

#include <FGFDMExec.h>
#include <input_output/FGGroundCallback.h>
#include <math/FGColumnVector3.h>
#include <math/FGLocation.h>
#include <models/FGInertial.h>

#include <cmath>
#include <cstddef>
#include <cstring>
#include <exception>
#include <memory>
#include <string>
#include <vector>

namespace {

constexpr double kFeetPerMetre = 1.0 / 0.3048;

// Answers the terrain height under (latitude, longitude) in radians:
// metres above the ellipsoid, and its slope north and east (m/m).
// Returns 0 when it has no answer.
using GroundFn = int (*)(void* ctx, double lat_rad, double lon_rad,
                         double* height_m, double* slope_north,
                         double* slope_east);

class RustGround : public JSBSim::FGGroundCallback {
 public:
  RustGround(GroundFn fn, void* ctx, double a, double b)
      : fn_(fn), ctx_(ctx), a_(a), b_(b) {}

  double GetAGLevel(double, const JSBSim::FGLocation& location,
                    JSBSim::FGLocation& contact,
                    JSBSim::FGColumnVector3& normal,
                    JSBSim::FGColumnVector3& v,
                    JSBSim::FGColumnVector3& w) const override {
    v.InitMatrix();
    w.InitMatrix();
    JSBSim::FGLocation l = location;
    l.SetEllipse(a_, b_);
    const double lat = l.GetGeodLatitudeRad();
    const double lon = l.GetLongitude();
    double height_m = last_height_m_;
    double slope_n = 0.0;
    double slope_e = 0.0;
    if (fn_(ctx_, lat, lon, &height_m, &slope_n, &slope_e) != 0) {
      last_height_m_ = height_m;
    }
    const double h_ft = height_m * kFeetPerMetre;
    const double sin_lat = std::sin(lat), cos_lat = std::cos(lat);
    const double sin_lon = std::sin(lon), cos_lon = std::cos(lon);
    // Local up, north, and east in the Earth-centred frame.
    const JSBSim::FGColumnVector3 up(cos_lat * cos_lon, cos_lat * sin_lon,
                                     sin_lat);
    const JSBSim::FGColumnVector3 north(-sin_lat * cos_lon,
                                        -sin_lat * sin_lon, cos_lat);
    const JSBSim::FGColumnVector3 east(-sin_lon, cos_lon, 0.0);
    normal = up - slope_n * north - slope_e * east;
    normal.Normalize();
    contact.SetEllipse(a_, b_);
    contact.SetPositionGeodetic(lon, lat, h_ft);
    return l.GetGeodAltitude() - h_ft;
  }

  void SetEllipse(double a, double b) override {
    a_ = a;
    b_ = b;
  }

 private:
  GroundFn fn_;
  void* ctx_;
  double a_;
  double b_;
  // The height to keep when Rust has no answer: a missing tile must not
  // drop the ground from under the vehicle.
  mutable double last_height_m_ = 0.0;
};

void write_error(char* err, std::size_t err_len, const char* message) {
  if (err == nullptr || err_len == 0) return;
  std::strncpy(err, message, err_len - 1);
  err[err_len - 1] = '\0';
}

}  // namespace

struct PjsEngine {
  std::unique_ptr<JSBSim::FGFDMExec> exec;
  std::vector<SGPropertyNode*> nodes;
};

extern "C" {

PjsEngine* pjs_create(const char* root, const char* aircraft_dir,
                      const char* engine_dir, const char* systems_dir,
                      const char* model, double dt, char* err,
                      std::size_t err_len) {
  try {
    auto engine = std::make_unique<PjsEngine>();
    engine->exec = std::make_unique<JSBSim::FGFDMExec>();
    engine->exec->SetDebugLevel(0);
    engine->exec->SetRootDir(SGPath(root));
    if (!engine->exec->LoadModel(SGPath(aircraft_dir), SGPath(engine_dir),
                                 SGPath(systems_dir), std::string(model),
                                 false)) {
      write_error(err, err_len, "JSBSim could not load the aircraft model");
      return nullptr;
    }
    engine->exec->Setdt(dt);
    return engine.release();
  } catch (const std::exception& e) {
    write_error(err, err_len, e.what());
  } catch (...) {
    write_error(err, err_len, "JSBSim threw a non-standard exception");
  }
  return nullptr;
}

void pjs_destroy(PjsEngine* engine) { delete engine; }

int pjs_set_ground(PjsEngine* engine, GroundFn fn, void* ctx) {
  try {
    auto inertial = engine->exec->GetInertial();
    inertial->SetGroundCallback(new RustGround(
        fn, ctx, inertial->GetSemimajor(), inertial->GetSemiminor()));
    return 1;
  } catch (...) {
    return 0;
  }
}

int pjs_property(PjsEngine* engine, const char* name, int create) {
  try {
    SGPropertyNode* node =
        engine->exec->GetPropertyManager()->GetNode(name, create != 0);
    if (node == nullptr) return -1;
    engine->nodes.push_back(node);
    return static_cast<int>(engine->nodes.size() - 1);
  } catch (...) {
    return -1;
  }
}

// An index outside the table reads NaN and writes nothing: the Rust side
// only holds indices this engine returned, so it is a defect, not data.
// A tied property runs JSBSim code (a trim, for one), which may throw.
double pjs_get(const PjsEngine* engine, int index) {
  if (index < 0 || static_cast<std::size_t>(index) >= engine->nodes.size()) {
    return std::nan("");
  }
  try {
    return engine->nodes[static_cast<std::size_t>(index)]->getDoubleValue();
  } catch (...) {
    return std::nan("");
  }
}

int pjs_set(PjsEngine* engine, int index, double value) {
  if (index < 0 || static_cast<std::size_t>(index) >= engine->nodes.size()) {
    return 0;
  }
  try {
    return engine->nodes[static_cast<std::size_t>(index)]->setDoubleValue(value)
               ? 1
               : 0;
  } catch (...) {
    return 0;
  }
}

int pjs_run_ic(PjsEngine* engine) {
  try {
    return engine->exec->RunIC() ? 1 : 0;
  } catch (...) {
    return 0;
  }
}

int pjs_run(PjsEngine* engine) {
  try {
    return engine->exec->Run() ? 1 : 0;
  } catch (...) {
    return 0;
  }
}

double pjs_sim_time(const PjsEngine* engine) {
  return engine->exec->GetSimTime();
}

}  // extern "C"
