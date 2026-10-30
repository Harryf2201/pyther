"""Compare paired test sensitivity without confusing logistic and normalized Elo.

Stdlib only. Counts are LL,LD,DD/WL,WD,WW from the DEV perspective.
LLR is a Gaussian small-effect approximation for normalized [0,5] bounds;
it is a diagnostic, not the exact constrained pentanomial GSPRT.
Definitions: https://github.com/official-stockfish/fishtest/blob/master/server/fishtest/stats/LLRcalc.py
"""
import argparse
import json
import math

def measure(counts, lo=0, hi=5):
    pairs = sum(counts)
    if pairs<=0 or any(n<0 for n in counts): raise ValueError('nonnegative, nonempty counts required')
    mean = sum(i*n/4 for i,n in enumerate(counts))/pairs
    variance = sum(n*(i/4-mean)**2 for i,n in enumerate(counts))/pairs
    if variance<=0: raise ValueError('nonzero pair variance required')
    games = 2*pairs
    k = 800/math.log(10)
    normalized = k*(mean-.5)/math.sqrt(2*variance)
    llr_per_game = ((hi-lo)*normalized-(hi*hi-lo*lo)/2)/(k*k)
    return dict(games=games,counts=counts,mean_score=mean,pair_variance=variance,
        logistic_elo=400*math.log10(mean/(1-mean)) if 0<mean<1 else None,
        normalized_elo=normalized,normalized_95_halfwidth_approx=1.96*k/math.sqrt(games),
        normalized_bounds=[lo,hi],llr_approx=games*llr_per_game,llr_per_game_approx=llr_per_game,
        z=(mean-.5)*math.sqrt(pairs/variance),center_pair_fraction=counts[2]/pairs)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--counts',type=int,nargs=5,required=True)
    p.add_argument('--flip',action='store_true',help='reverse dev/base so an expected loss becomes positive')
    p.add_argument('--bounds',type=float,nargs=2,default=[0,5])
    a=p.parse_args()
    print(json.dumps(measure(a.counts[::-1] if a.flip else a.counts,*a.bounds),indent=2))
