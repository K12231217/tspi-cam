// Reference runner: upstream C++ VQF (dlaidig/vqf) in 6D mode.
// Produces the expected outputs the Rust port is validated against.
// Build: g++ -O2 -std=c++17 ref_driver.cpp vqf.cpp -o ref_driver
//   (vqf.hpp / vqf.cpp from https://github.com/dlaidig/vqf, MIT)
// Usage: ref_driver <input.csv> <quat6d_out.csv> <bias_out.csv> <ts>
// input.csv rows: gx,gy,gz,ax,ay,az (rad/s, m/s^2).
// quat6d_out.csv rows: w,x,y,z per sample. bias_out.csv: one row
// bx,by,bz,sigma (rad/s) at the end of the run.
#include "vqf.hpp"

#include <array>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

int main(int argc, char** argv) {
    if (argc != 5) {
        std::fprintf(stderr, "usage: %s in.csv quat.csv bias.csv ts\n", argv[0]);
        return 2;
    }
    const double ts = std::atof(argv[4]);
    std::ifstream in(argv[1]);
    if (!in) {
        std::fprintf(stderr, "cannot open %s\n", argv[1]);
        return 2;
    }
    std::vector<std::array<double, 6>> samples;
    std::string line;
    while (std::getline(in, line)) {
        if (line.empty()) continue;
        std::stringstream ss(line);
        std::array<double, 6> s{};
        char comma = 0;
        for (int i = 0; i < 6; i++) {
            ss >> s[i];
            if (i < 5) ss >> comma;
        }
        samples.push_back(s);
    }

    VQF vqf(ts);
    std::ofstream qout(argv[2]);
    qout.precision(17);
    for (const auto& s : samples) {
        const double gyr[3] = {s[0], s[1], s[2]};
        const double acc[3] = {s[3], s[4], s[5]};
        vqf.update(gyr, acc);
        double q[4];
        vqf.getQuat6D(q);
        qout << q[0] << ',' << q[1] << ',' << q[2] << ',' << q[3] << '\n';
    }
    double bias[3];
    const double sigma = vqf.getBiasEstimate(bias);
    std::ofstream bout(argv[3]);
    bout.precision(17);
    bout << bias[0] << ',' << bias[1] << ',' << bias[2] << ',' << sigma << '\n';
    std::fprintf(stderr, "ran %zu samples, ts=%g\n", samples.size(), ts);
    return 0;
}
